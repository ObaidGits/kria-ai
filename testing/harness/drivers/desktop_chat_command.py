from __future__ import annotations

import json
import time
import urllib.error
import urllib.request
from typing import Any

from testing.harness.assertions.chat_response import assert_chat_response
from testing.harness.drivers.chat_api import (
    _extract_variables,
    _int_list,
    _object_list,
    _response_evidence_preview,
    _scenario_steps,
    _substitute,
    _substitute_value,
    _verification_failure_class,
    health_check,
    resolve_chat_base_url,
    _read_token,
)
from testing.harness.drivers.n8n_api import (
    delete_disposable_workflows_by_prefix,
    delete_workflow_if_disposable,
    resolve_n8n_base_url,
    run_n8n_action,
    run_n8n_verification,
)
from testing.harness.models import RunContext, Scenario, ScenarioResult
from testing.harness.reporting.redaction import redact_json


GENERIC_N8N_REFUSALS = [
    "i cannot create workflows",
    "cannot create workflows",
    "cannot create or modify n8n workflows",
    "only n8n-related tool",
    "don't have a tool to archive",
    "don't have a tool to delete",
    "i can help you design this workflow",
    "build it yourself in n8n",
]


def _now_ms() -> int:
    return int(time.time() * 1000)


def send_desktop_chat_command(
    *,
    base_url: str,
    message: str,
    session_id: str,
    timeout_seconds: int = 120,
) -> dict[str, Any]:
    body = json.dumps({"message": message, "session_id": session_id}).encode("utf-8")
    headers = {"Content-Type": "application/json", "Accept": "application/json"}
    token = _read_token(base_url)
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}/api/testing/desktop-chat-command",
        data=body,
        headers=headers,
        method="POST",
    )
    started = time.time()
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            payload = response.read().decode("utf-8", errors="replace")
            try:
                parsed = json.loads(payload) if payload else {}
            except json.JSONDecodeError:
                parsed = {"raw": payload}
            return {
                "ok": 200 <= response.status < 300,
                "status_code": response.status,
                "duration_ms": int((time.time() - started) * 1000),
                "response": redact_json(parsed),
            }
    except urllib.error.HTTPError as error:
        payload = error.read().decode("utf-8", errors="replace")
        try:
            parsed = json.loads(payload) if payload else {}
        except json.JSONDecodeError:
            parsed = {"error": payload}
        return {
            "ok": False,
            "status_code": error.code,
            "duration_ms": int((time.time() - started) * 1000),
            "response": redact_json(parsed),
        }
    except OSError as error:
        return {
            "ok": False,
            "status_code": None,
            "duration_ms": int((time.time() - started) * 1000),
            "response": {"error": str(error)},
        }


def run_desktop_chat_command_scenario(scenario: Scenario, context: RunContext) -> ScenarioResult:
    started_ms = _now_ms()
    inputs = scenario.inputs
    base_url = resolve_chat_base_url(context.root_dir, inputs)
    health = health_check(base_url)
    if not health.get("ok"):
        ended_ms = _now_ms()
        return ScenarioResult(
            scenario_id=scenario.id,
            title=scenario.title,
            status="blocked",
            verdict="blocked",
            failure_class="environment",
            started_at_ms=started_ms,
            ended_at_ms=ended_ms,
            duration_ms=ended_ms - started_ms,
            tags=scenario.tags,
            required_services=scenario.required_services,
            evidence=[{"type": "desktop_chat_command_health", "base_url": base_url, "result": health}],
            failure={"message": f"KRIA Desktop local API is not reachable at {base_url}"},
        )

    variables: dict[str, Any] = {
        "run_id": context.run_id,
        "scenario_id": scenario.id,
    }
    variables["run_prefix"] = _substitute(
        str(inputs.get("disposable_prefix") or f"KRIA Desktop Command E2E {context.run_id}"),
        variables,
    )
    steps = _scenario_steps(inputs)
    evidence: list[dict[str, Any]] = [
        {"type": "desktop_chat_command_health", "base_url": base_url, "result": health}
    ]
    failures: list[str] = []
    failure_class = "assertion"

    for index, step in enumerate(steps, start=1):
        if not isinstance(step, dict):
            failures.append(f"step {index} is not an object")
            continue
        prompt_template = str(step.get("prompt") or "")
        if not prompt_template:
            failures.append(f"step {index} is missing prompt")
            continue
        blocked = False
        for action in _object_list(step.get("n8n_actions", [])):
            ok, message, detail, outputs = run_n8n_action(
                action=action,
                variables=variables,
                root_dir=context.root_dir,
            )
            variables.update(outputs)
            evidence.append(
                {
                    "type": "n8n_action",
                    "step": index,
                    "ok": ok,
                    "message": message,
                    "detail": detail,
                }
            )
            if not ok:
                failure_class = _verification_failure_class(detail)
                failures.append(f"step {index}: {message}")
                if not step.get("continue_after_n8n_action_failure"):
                    blocked = True
                    break
        if blocked:
            continue

        prompt = _substitute(prompt_template, variables)
        session_id = _substitute(
            str(step.get("session_id") or inputs.get("session_id") or f"{scenario.id}-{context.run_id}-{index}"),
            variables,
        )
        response = send_desktop_chat_command(
            base_url=base_url,
            message=prompt,
            session_id=session_id,
            timeout_seconds=int(step.get("timeout_seconds") or scenario.timeout_seconds),
        )
        parsed_response = response.get("response")
        event_names = _event_names(parsed_response)
        evidence.append(
            {
                "type": "desktop_chat_command_step",
                "step": index,
                "prompt_preview": prompt[:240],
                "status_code": response.get("status_code"),
                "duration_ms": response.get("duration_ms"),
                "event_names": event_names,
                "response": _desktop_response_evidence_preview(parsed_response),
            }
        )

        expected_http_statuses = _int_list(step.get("expected_http_status_any", []))
        http_status_allowed = response.get("status_code") in expected_http_statuses
        if not response.get("ok") and not http_status_allowed:
            status_code = response.get("status_code")
            failure_class = "environment" if status_code is None else "product"
            failures.append(f"step {index} desktop chat command failed with status {status_code}")
            continue
        if not isinstance(parsed_response, dict):
            failures.append(f"step {index} response was not a JSON object")
            continue

        desktop_meta = parsed_response.get("desktop_command") if isinstance(parsed_response.get("desktop_command"), dict) else {}
        if desktop_meta.get("path") != "send_message" or desktop_meta.get("ui_opened") is not False:
            failures.append("desktop command response did not prove non-UI send_message path")
        if parsed_response.get("status") != "not_handled":
            for required in ("agent:thinking", "agent:token", "agent:tool_result", "agent:done"):
                if required not in event_names:
                    failures.append(f"step {index}: missing Desktop chat event {required}")

        generic_refusal = _generic_refusal(parsed_response)
        if generic_refusal:
            failures.append(f"step {index}: generic n8n refusal appeared: {generic_refusal}")

        assertion_response = _assertion_response(parsed_response)
        assertion_spec = _substitute_value(step, variables)
        failures.extend(
            f"step {index}: {failure}"
            for failure in assert_chat_response(assertion_response, assertion_spec)
        )
        _extract_variables(assertion_response, assertion_spec.get("extract", {}), variables)

        post_blocked = False
        for action in _object_list(step.get("post_n8n_actions", [])):
            ok, message, detail, outputs = run_n8n_action(
                action=action,
                variables=variables,
                root_dir=context.root_dir,
            )
            variables.update(outputs)
            evidence.append(
                {
                    "type": "post_n8n_action",
                    "step": index,
                    "ok": ok,
                    "message": message,
                    "detail": detail,
                }
            )
            if not ok:
                failure_class = _verification_failure_class(detail)
                failures.append(f"step {index}: {message}")
                if not step.get("continue_after_post_n8n_action_failure"):
                    post_blocked = True
                    break
        if post_blocked:
            continue

        for verification in step.get("n8n_verifications", []):
            if not isinstance(verification, dict):
                failures.append(f"step {index}: n8n verification is not an object")
                continue
            ok, message, detail = run_n8n_verification(
                verification=verification,
                variables=variables,
                response=assertion_response,
                root_dir=context.root_dir,
            )
            evidence.append(
                {
                    "type": "n8n_verification",
                    "step": index,
                    "ok": ok,
                    "message": message,
                    "detail": detail,
                }
            )
            if not ok:
                failure_class = _verification_failure_class(detail)
                failures.append(f"step {index}: {message}")

    cleanup = _cleanup_created_resources_desktop(base_url, inputs, variables, context)
    ended_ms = _now_ms()
    if failures:
        return ScenarioResult(
            scenario_id=scenario.id,
            title=scenario.title,
            status="failed" if failure_class != "environment" else "infra_failed",
            verdict="failed",
            failure_class=failure_class,
            started_at_ms=started_ms,
            ended_at_ms=ended_ms,
            duration_ms=ended_ms - started_ms,
            tags=scenario.tags,
            required_services=scenario.required_services,
            evidence=evidence,
            cleanup=cleanup,
            failure={"message": "; ".join(failures[:5]), "all_failures": failures},
        )
    return ScenarioResult(
        scenario_id=scenario.id,
        title=scenario.title,
        status="passed",
        verdict="passed",
        failure_class=None,
        started_at_ms=started_ms,
        ended_at_ms=ended_ms,
        duration_ms=ended_ms - started_ms,
        tags=scenario.tags,
        required_services=scenario.required_services,
        evidence=evidence,
        cleanup=cleanup,
    )


def _event_names(response: Any) -> list[str]:
    if not isinstance(response, dict):
        return []
    events = response.get("events")
    if not isinstance(events, list):
        return []
    names = []
    for event in events:
        if isinstance(event, dict) and isinstance(event.get("name"), str):
            names.append(event["name"])
    return names


def _assertion_response(response: dict[str, Any]) -> dict[str, Any]:
    nested = response.get("response") if isinstance(response.get("response"), dict) else {}
    merged = dict(nested)
    if not merged or response.get("status") == "not_handled":
        for key in ("status", "reply"):
            if key in response:
                merged[key] = response[key]
    else:
        for key in ("status", "reply"):
            if key in response:
                merged.setdefault(key, response[key])
    for key in ("events", "desktop_command"):
        if key in response:
            merged[key] = response[key]
    if "n8n" not in merged and isinstance(nested.get("n8n"), dict):
        merged["n8n"] = nested["n8n"]
    return merged


def _generic_refusal(response: dict[str, Any]) -> str:
    text = json.dumps(response, sort_keys=True).lower()
    for phrase in GENERIC_N8N_REFUSALS:
        if phrase in text:
            return phrase
    return ""


def _desktop_response_evidence_preview(value: Any) -> Any:
    if not isinstance(value, dict):
        return redact_json(value)
    preview = {
        "status": value.get("status"),
        "reply": str(value.get("reply") or "")[:600],
        "desktop_command": value.get("desktop_command"),
        "event_names": _event_names(value),
        "response": _response_evidence_preview(_assertion_response(value)),
    }
    return redact_json(preview)


def _cleanup_created_resources_desktop(
    base_url: str, inputs: dict[str, Any], variables: dict[str, Any], context: RunContext
) -> dict[str, Any]:
    cleanup_spec = inputs.get("cleanup", {})
    if not isinstance(cleanup_spec, dict):
        return {"status": "not_required", "actions": []}
    actions: list[dict[str, Any]] = []
    failed = False

    if cleanup_spec.get("cleanup_kria_draft_workflow_ids", True):
        workflow_ids = _unique_values(
            variables.get(name)
            for name in cleanup_spec.get("workflow_id_vars", ["workflow_id", "copy_workflow_id"])
        )
        for workflow_id in workflow_ids:
            message = f"Cleanup draft {workflow_id}"
            if cleanup_spec.get("delete_n8n_draft", True):
                message += " and delete n8n draft"
            response = send_desktop_chat_command(
                base_url=base_url,
                message=message,
                session_id=f"desktop-command-cleanup-{context.run_id}-{workflow_id}",
                timeout_seconds=120,
            )
            ok = bool(response.get("ok")) and str(
                (response.get("response") or {}).get("status") or ""
            ) in {"cleaned_up", "removed", "ok", "accepted", "processing"}
            actions.append(
                {
                    "kind": "desktop_command_cleanup_kria_draft",
                    "workflow_id": workflow_id,
                    "status": "passed" if ok else "failed",
                    "response": _desktop_response_evidence_preview(response.get("response")),
                }
            )
            failed = failed or not ok

    if cleanup_spec.get("delete_disposable_n8n_workflow_ids"):
        n8n_base_url = resolve_n8n_base_url(context.root_dir)
        workflow_ids = _unique_values(
            variables.get(name)
            for name in cleanup_spec.get("n8n_workflow_id_vars", ["n8n_workflow_id"])
        )
        for workflow_id in workflow_ids:
            created = variables.get("_created_n8n_workflow_ids")
            result = delete_workflow_if_disposable(
                base_url=n8n_base_url,
                workflow_id=str(workflow_id),
                allow_workflow_ids=[str(item) for item in created] if isinstance(created, list) else None,
            )
            actions.append(
                {
                    "kind": "delete_disposable_n8n_workflow",
                    "workflow_id": workflow_id,
                    "status": "passed" if result.get("ok") else "failed",
                    "result": result,
                }
            )
            failed = failed or not result.get("ok")

    if cleanup_spec.get("delete_disposable_n8n_run_prefix"):
        n8n_base_url = resolve_n8n_base_url(context.root_dir)
        prefix = _substitute(str(cleanup_spec.get("prefix") or variables.get("run_prefix") or ""), variables)
        result = delete_disposable_workflows_by_prefix(base_url=n8n_base_url, prefix=prefix)
        actions.append(
            {
                "kind": "delete_disposable_n8n_run_prefix",
                "prefix": prefix,
                "status": "passed" if result.get("ok") else "failed",
                "result": result,
            }
        )
        failed = failed or not result.get("ok")

    if not actions:
        return {"status": "not_required", "actions": []}
    return {"status": "failed" if failed else "passed", "actions": actions}


def _unique_values(values: Any) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        if value in (None, ""):
            continue
        item = str(value)
        if item not in seen:
            seen.add(item)
            result.append(item)
    return result
