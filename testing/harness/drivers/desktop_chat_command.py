from __future__ import annotations

import json
import time
import urllib.error
import urllib.request
from socket import timeout as SocketTimeout
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


def _progress(message: str) -> None:
    print(f"[desktop_chat_command] {message}", flush=True)


def send_desktop_chat_command(
    *,
    base_url: str,
    message: str,
    session_id: str,
    manual_profile: dict[str, Any] | None = None,
    gui_cognition_test: dict[str, Any] | None = None,
    timeout_seconds: int = 120,
) -> dict[str, Any]:
    payload: dict[str, Any] = {"message": message, "session_id": session_id}
    if manual_profile is not None:
        payload["manual_profile"] = manual_profile
    if gui_cognition_test is not None:
        payload["gui_cognition_test"] = gui_cognition_test
    body = json.dumps(payload).encode("utf-8")
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
    except (TimeoutError, SocketTimeout) as error:
        return {
            "ok": False,
            "status_code": None,
            "duration_ms": int((time.time() - started) * 1000),
            "timed_out": True,
            "timeout_seconds": timeout_seconds,
            "response": {"error": f"desktop command request timed out: {error}"},
        }
    except OSError as error:
        error_text = str(error)
        timed_out = "timed out" in error_text.lower() or "timeout" in error_text.lower()
        return {
            "ok": False,
            "status_code": None,
            "duration_ms": int((time.time() - started) * 1000),
            "timed_out": timed_out,
            "timeout_seconds": timeout_seconds,
            "response": {"error": error_text},
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
        manual_profile = step.get("manual_profile", inputs.get("manual_profile"))
        if manual_profile is not None:
            manual_profile = _substitute_value(manual_profile, variables)
            if not isinstance(manual_profile, dict):
                failures.append(f"step {index}: manual_profile must be an object when provided")
                continue
        gui_cognition_test = step.get("gui_cognition_test", inputs.get("gui_cognition_test"))
        if gui_cognition_test is not None:
            gui_cognition_test = _substitute_value(gui_cognition_test, variables)
            if not isinstance(gui_cognition_test, dict):
                failures.append(f"step {index}: gui_cognition_test must be an object when provided")
                continue
        step_timeout_seconds = int(step.get("timeout_seconds") or scenario.timeout_seconds)
        step_started_at_ms = _now_ms()
        manual_profile_mode_id = (
            str(manual_profile.get("mode_id"))
            if isinstance(manual_profile, dict) and manual_profile.get("mode_id") is not None
            else None
        )
        fixture_name = _gui_cognition_fixture_name(gui_cognition_test)
        _progress(
            f"{scenario.id} step {index}/{len(steps)} POST {base_url} "
            f"timeout={step_timeout_seconds}s mode={manual_profile_mode_id or 'auto'} "
            f"fixture={fixture_name or 'none'} prompt={prompt[:120]!r}"
        )
        response = send_desktop_chat_command(
            base_url=base_url,
            message=prompt,
            session_id=session_id,
            manual_profile=manual_profile,
            gui_cognition_test=gui_cognition_test,
            timeout_seconds=step_timeout_seconds,
        )
        _progress(
            f"{scenario.id} step {index}/{len(steps)} completed "
            f"status={response.get('status_code')} ok={response.get('ok')} "
            f"elapsed_ms={response.get('duration_ms')}"
        )
        parsed_response = response.get("response")
        event_names = _event_names(parsed_response)
        gui_event_types = _gui_event_types(parsed_response)
        observation_timing = _gui_observation_timing(parsed_response)
        evidence.append(
            {
                "type": "desktop_chat_command_step",
                "step": index,
                "step_started_at_ms": step_started_at_ms,
                "step_timeout_seconds": step_timeout_seconds,
                "base_url": base_url,
                "manual_profile_mode_id": manual_profile_mode_id,
                "gui_cognition_test": redact_json(gui_cognition_test),
                "prompt_preview": prompt[:240],
                "status_code": response.get("status_code"),
                "duration_ms": response.get("duration_ms"),
                "timed_out": bool(response.get("timed_out")),
                "event_names": event_names,
                "gui_event_types": gui_event_types,
                "observation_timing": observation_timing,
                "response": _desktop_response_evidence_preview(parsed_response),
            }
        )

        expected_http_statuses = _int_list(step.get("expected_http_status_any", []))
        http_status_allowed = response.get("status_code") in expected_http_statuses
        if not response.get("ok") and not http_status_allowed:
            status_code = response.get("status_code")
            if response.get("timed_out"):
                failure_class = "harness"
                failures.append(
                    f"step {index} desktop command request timed out after {step_timeout_seconds}s"
                )
            else:
                failure_class = "environment" if status_code is None else "product"
                failures.append(f"step {index} desktop chat command failed with status {status_code}")
            continue
        if not isinstance(parsed_response, dict):
            failures.append(f"step {index} response was not a JSON object")
            continue

        desktop_meta = parsed_response.get("desktop_command") if isinstance(parsed_response.get("desktop_command"), dict) else {}
        expected_desktop_path = str(
            step.get("expected_desktop_path")
            or inputs.get("expected_desktop_path")
            or "send_message"
        )
        if desktop_meta.get("path") != expected_desktop_path or desktop_meta.get("ui_opened") is not False:
            failures.append(
                "desktop command response did not prove non-UI "
                f"{expected_desktop_path} path"
            )
        if parsed_response.get("status") != "not_handled":
            for required in ("agent:thinking", "agent:token", "agent:tool_result", "agent:done"):
                if required not in event_names:
                    failures.append(f"step {index}: missing Desktop chat event {required}")
        for required in _string_list(step.get("expected_event_names", inputs.get("expected_event_names", []))):
            if required not in event_names:
                failures.append(f"step {index}: missing expected Desktop chat event {required}")
        for forbidden in _string_list(step.get("forbidden_event_names", inputs.get("forbidden_event_names", []))):
            if forbidden in event_names:
                failures.append(f"step {index}: forbidden Desktop chat event appeared: {forbidden}")
        for required in _string_list(step.get("expected_gui_event_types", inputs.get("expected_gui_event_types", []))):
            if required not in gui_event_types:
                failures.append(f"step {index}: missing GUI cognition event type {required}")
        for forbidden in _string_list(step.get("forbidden_gui_event_types", inputs.get("forbidden_gui_event_types", []))):
            if forbidden in gui_event_types:
                failures.append(f"step {index}: forbidden GUI cognition event type appeared: {forbidden}")
        if bool(step.get("assert_gui_event_sequence_monotonic", inputs.get("assert_gui_event_sequence_monotonic", False))):
            sequences = _gui_event_sequences(parsed_response)
            if sequences != sorted(sequences) or len(sequences) != len(set(sequences)):
                failures.append(f"step {index}: GUI cognition event sequence is not monotonic")

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


def _gui_event_types(response: Any) -> list[str]:
    events = _gui_events(response)
    types: list[str] = []
    for event in events:
        payload = event.get("payload") if isinstance(event, dict) else None
        inner = payload.get("event") if isinstance(payload, dict) else None
        event_type = inner.get("type") if isinstance(inner, dict) else None
        if isinstance(event_type, str):
            types.append(event_type)
    return types


def _gui_event_sequences(response: Any) -> list[int]:
    events = _gui_events(response)
    sequences: list[int] = []
    for event in events:
        payload = event.get("payload") if isinstance(event, dict) else None
        sequence = payload.get("sequence") if isinstance(payload, dict) else None
        if isinstance(sequence, int):
            sequences.append(sequence)
    return sequences


def _gui_observation_timing(response: Any) -> dict[str, Any]:
    if not isinstance(response, dict):
        return {}
    assertion_response = _assertion_response(response)
    gui = assertion_response.get("gui_cognition")
    if isinstance(gui, dict):
        perception = gui.get("perception")
        if isinstance(perception, dict) and isinstance(perception.get("observation_total_ms"), (int, float)):
            return {
                "observation_total_ms": perception.get("observation_total_ms"),
                "slowest_probe": perception.get("slowest_probe"),
                "slowest_probe_ms": perception.get("slowest_probe_ms"),
                "probe_timeout_count": perception.get("probe_timeout_count"),
                "cache_hit": perception.get("cache_hit"),
                "cache_age_ms": perception.get("cache_age_ms"),
            }
    for event in _gui_events(response):
        payload = event.get("payload") if isinstance(event, dict) else None
        inner = payload.get("event") if isinstance(payload, dict) else None
        if isinstance(inner, dict) and inner.get("type") == "ObservationCompleted":
            return {
                "observation_total_ms": inner.get("observation_total_ms"),
                "slowest_probe": inner.get("slowest_probe"),
                "slowest_probe_ms": inner.get("slowest_probe_ms"),
                "probe_timeout_count": inner.get("probe_timeout_count"),
                "cache_hit": inner.get("cache_hit"),
                "cache_age_ms": inner.get("cache_age_ms"),
            }
    return {}


def _gui_events(response: Any) -> list[dict[str, Any]]:
    if not isinstance(response, dict):
        return []
    events = response.get("events")
    if not isinstance(events, list):
        return []
    return [
        event
        for event in events
        if isinstance(event, dict) and event.get("name") == "gui_cognition:event"
    ]


def _string_list(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return [str(item) for item in value if isinstance(item, str)]


def _gui_cognition_fixture_name(value: Any) -> str | None:
    if not isinstance(value, dict):
        return None
    for key in (
        "llm_planner_fixture",
        "planner_fixture",
        "perception_fixture",
        "action_backend_fixture",
        "verifier_fixture",
        "recovery_fixture",
        "resume_fixture",
    ):
        fixture = value.get(key)
        if isinstance(fixture, str) and fixture:
            return f"{key}={fixture}"
    return "custom" if value else None


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
