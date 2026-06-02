from __future__ import annotations

import json
import os
import time
import tomllib
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from testing.harness.assertions.chat_response import assert_chat_response, first_path_value
from testing.harness.drivers.n8n_api import (
    delete_disposable_workflows_by_prefix,
    delete_workflow_if_disposable,
    resolve_n8n_base_url,
    run_n8n_action,
    run_n8n_verification,
)
from testing.harness.models import RunContext, Scenario, ScenarioResult
from testing.harness.reporting.redaction import redact_json


def _now_ms() -> int:
    return int(time.time() * 1000)


def _read_token(base_url: str | None = None) -> str:
    env_token = os.environ.get("KRIA_API_TOKEN", "").strip()
    if env_token:
        return env_token
    token_path = Path.home() / ".kria" / "api_token"
    if token_path.exists():
        return token_path.read_text(encoding="utf-8").strip()
    if base_url:
        token = get_api_token(base_url)
        if token:
            return token
    return ""


def resolve_chat_base_url(root_dir: Path, inputs: dict[str, Any]) -> str:
    env_name = str(inputs.get("base_url_env") or "KRIA_API_BASE_URL")
    env_value = os.environ.get(env_name, "").strip()
    if env_value:
        return env_value
    explicit = str(inputs.get("base_url") or "").strip()
    if explicit:
        return explicit
    config_path = root_dir / "config" / "default.toml"
    if config_path.exists():
        try:
            data = tomllib.loads(config_path.read_text(encoding="utf-8"))
            server = data.get("server", {})
            host = str(server.get("host") or "127.0.0.1")
            port = int(server.get("port") or 3001)
            if host in {"0.0.0.0", "::"}:
                host = "127.0.0.1"
            return f"http://{host}:{port}"
        except (OSError, ValueError, tomllib.TOMLDecodeError):
            pass
    return "http://127.0.0.1:3001"


def get_api_token(base_url: str, timeout_seconds: int = 5) -> str:
    try:
        request = urllib.request.Request(
            f"{base_url.rstrip('/')}/api/auth/token",
            headers={"Accept": "application/json"},
            method="GET",
        )
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            payload = response.read().decode("utf-8", errors="replace")
            parsed = json.loads(payload)
            return str(parsed.get("token") or "").strip()
    except (OSError, json.JSONDecodeError):
        return ""


def health_check(base_url: str, timeout_seconds: int = 5) -> dict[str, Any]:
    try:
        request = urllib.request.Request(
            f"{base_url.rstrip('/')}/api/health",
            headers={"Accept": "application/json"},
            method="GET",
        )
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            payload = response.read().decode("utf-8", errors="replace")
            try:
                parsed = json.loads(payload)
            except json.JSONDecodeError:
                parsed = {"raw": payload}
            return {
                "ok": 200 <= response.status < 500,
                "status_code": response.status,
                "response": redact_json(parsed),
            }
    except OSError as error:
        return {"ok": False, "status_code": None, "error": str(error)}


def send_chat_message(
    *,
    base_url: str,
    message: str,
    session_id: str,
    source: str = "testing_spine",
    from_user: str = "testing-spine",
    timeout_seconds: int = 120,
) -> dict[str, Any]:
    body = json.dumps(
        {
            "message": message,
            "session_id": session_id,
            "source": source,
            "from_user": from_user,
        }
    ).encode("utf-8")
    headers = {"Content-Type": "application/json"}
    token = _read_token(base_url)
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}/api/chat", data=body, headers=headers, method="POST"
    )
    started = time.time()
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            payload = response.read().decode("utf-8", errors="replace")
            try:
                parsed = json.loads(payload)
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
            parsed = json.loads(payload)
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


def run_chat_api_scenario(scenario: Scenario, context: RunContext) -> ScenarioResult:
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
            evidence=[{"type": "chat_api_health", "base_url": base_url, "result": health}],
            failure={"message": f"KRIA local API is not reachable at {base_url}"},
        )

    variables: dict[str, Any] = {
        "run_id": context.run_id,
        "scenario_id": scenario.id,
        "run_prefix": inputs.get("disposable_prefix", f"KRIA E2E Test {context.run_id}"),
    }
    steps = _scenario_steps(inputs)
    if not isinstance(steps, list) or not steps:
        steps = [inputs]

    evidence: list[dict[str, Any]] = [
        {"type": "chat_api_health", "base_url": base_url, "result": health}
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
        prompt = _substitute(prompt_template, variables)
        session_id = _substitute(
            str(
                step.get("session_id")
                or inputs.get("session_id")
                or f"{scenario.id}-{context.run_id}-{index}"
            ),
            variables,
        )
        step_blocked = False
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
                    step_blocked = True
                    break
        if step_blocked:
            continue

        response = send_chat_message(
            base_url=base_url,
            message=prompt,
            session_id=session_id,
            source=str(step.get("source") or inputs.get("source") or "n8n_prompt_e2e_native"),
            from_user=str(step.get("from_user") or inputs.get("from_user") or "prompt-eval"),
            timeout_seconds=int(step.get("timeout_seconds") or scenario.timeout_seconds),
        )
        evidence.append(
            {
                "type": "chat_api_step",
                "step": index,
                "prompt_preview": prompt[:240],
                "status_code": response.get("status_code"),
                "duration_ms": response.get("duration_ms"),
                "response": _response_evidence_preview(response.get("response")),
            }
        )
        expected_http_statuses = _int_list(step.get("expected_http_status_any", []))
        http_status_allowed = response.get("status_code") in expected_http_statuses
        if not response.get("ok") and not http_status_allowed:
            status_code = response.get("status_code")
            failure_class = "environment" if status_code is None else "product"
            failures.append(f"step {index} chat request failed with status {status_code}")
            continue

        parsed_response = response.get("response")
        if not isinstance(parsed_response, dict):
            failures.append(f"step {index} response was not a JSON object")
            continue

        assertion_spec = _substitute_value(step, variables)
        failures.extend(
            f"step {index}: {failure}"
            for failure in assert_chat_response(parsed_response, assertion_spec)
        )
        _extract_variables(parsed_response, assertion_spec.get("extract", {}), variables)

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
                response=parsed_response,
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

    cleanup = _cleanup_created_resources(base_url, inputs, variables, context)
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


def _extract_variables(
    response: dict[str, Any], extract_spec: Any, variables: dict[str, Any]
) -> None:
    if not isinstance(extract_spec, dict):
        return
    for name, paths in extract_spec.items():
        path_list = paths if isinstance(paths, list) else [paths]
        value = first_path_value(response, [str(path) for path in path_list])
        if value not in (None, ""):
            variables[str(name)] = value
            if str(name).endswith("n8n_workflow_id"):
                created = variables.setdefault("_created_n8n_workflow_ids", [])
                if isinstance(created, list) and str(value) not in created:
                    created.append(str(value))


def _substitute(value: str, variables: dict[str, Any]) -> str:
    result = value
    for key, item in variables.items():
        result = result.replace("${" + key + "}", str(item))
    return result


def _substitute_value(value: Any, variables: dict[str, Any]) -> Any:
    if isinstance(value, str):
        return _substitute(value, variables)
    if isinstance(value, list):
        return [_substitute_value(item, variables) for item in value]
    if isinstance(value, dict):
        return {key: _substitute_value(item, variables) for key, item in value.items()}
    return value


def _scenario_steps(inputs: dict[str, Any]) -> list[Any]:
    steps: list[Any] = []
    for key, phase in (
        ("setup_steps", "setup"),
        ("steps", "main"),
        ("teardown_steps", "teardown"),
    ):
        value = inputs.get(key)
        if isinstance(value, list):
            for item in value:
                if isinstance(item, dict):
                    next_item = dict(item)
                    next_item.setdefault("phase", phase)
                    steps.append(next_item)
    if steps:
        return steps
    value = inputs.get("steps")
    return value if isinstance(value, list) else [inputs]


def _response_evidence_preview(value: Any) -> Any:
    if not isinstance(value, dict):
        return redact_json(value)
    n8n = value.get("n8n") if isinstance(value.get("n8n"), dict) else {}
    result = n8n.get("result") if isinstance(n8n, dict) and isinstance(n8n.get("result"), dict) else {}
    routing = n8n.get("routing") if isinstance(n8n, dict) and isinstance(n8n.get("routing"), dict) else {}
    workflow = result.get("workflow") if isinstance(result.get("workflow"), dict) else {}
    preview = {
        "status": value.get("status"),
        "reply": str(value.get("reply") or "")[:600],
        "n8n": {
            "action": n8n.get("action") if isinstance(n8n, dict) else None,
            "routing_status": routing.get("status") if isinstance(routing, dict) else None,
            "workflow_id": result.get("workflow_id") or workflow.get("workflow_id"),
            "n8n_workflow_id": result.get("n8n_workflow_id") or workflow.get("n8n_workflow_id"),
            "blockers": routing.get("blockers") if isinstance(routing, dict) else None,
            "next_actions": routing.get("next_actions") if isinstance(routing, dict) else None,
        },
    }
    return redact_json(preview)


def _int_list(value: Any) -> list[int]:
    if isinstance(value, list):
        result = []
        for item in value:
            try:
                result.append(int(item))
            except (TypeError, ValueError):
                continue
        return result
    if value not in (None, ""):
        try:
            return [int(value)]
        except (TypeError, ValueError):
            return []
    return []


def _object_list(value: Any) -> list[dict[str, Any]]:
    if isinstance(value, list):
        return [item for item in value if isinstance(item, dict)]
    if isinstance(value, dict):
        return [value]
    return []


def _cleanup_created_resources(
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
            _cleanup_kria_draft(
                actions=actions,
                base_url=base_url,
                context=context,
                workflow_id=workflow_id,
                delete_n8n_draft=bool(cleanup_spec.get("delete_n8n_draft", True)),
            )
        local_only_ids = _unique_values(
            variables.get(name)
            for name in cleanup_spec.get("workflow_id_vars_no_n8n_delete", [])
        )
        for workflow_id in local_only_ids:
            _cleanup_kria_draft(
                actions=actions,
                base_url=base_url,
                context=context,
                workflow_id=workflow_id,
                delete_n8n_draft=False,
            )
        failed = any(action.get("status") == "failed" for action in actions)

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


def _cleanup_kria_draft(
    *,
    actions: list[dict[str, Any]],
    base_url: str,
    context: RunContext,
    workflow_id: str,
    delete_n8n_draft: bool,
) -> None:
    message = f"Cleanup draft {workflow_id}"
    if delete_n8n_draft:
        message += " and delete n8n draft"
    response = send_chat_message(
        base_url=base_url,
        message=message,
        session_id=f"cleanup-{context.run_id}-{workflow_id}",
        source="n8n_prompt_e2e_native_cleanup",
        from_user="prompt-eval",
        timeout_seconds=120,
    )
    ok = bool(response.get("ok")) and str(
        (response.get("response") or {}).get("status") or ""
    ) in {"cleaned_up", "removed", "ok", "accepted"}
    actions.append(
        {
            "kind": "cleanup_kria_draft",
            "workflow_id": workflow_id,
            "delete_n8n_draft": delete_n8n_draft,
            "status": "passed" if ok else "failed",
            "response": _response_evidence_preview(response.get("response")),
        }
    )


def _unused_legacy_cleanup_block() -> None:
    # Kept as a guard against accidental broad cleanup rewrites in patches.
    return None


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


def _verification_failure_class(detail: dict[str, Any]) -> str:
    text = json.dumps(detail).lower()
    if "missing n8n api key" in text or "connection refused" in text or "timed out" in text:
        return "environment"
    return "assertion"
