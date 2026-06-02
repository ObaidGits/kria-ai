from __future__ import annotations

import json
from typing import Any

from testing.harness.assertions.json_path import get_path


GENERIC_CREATE_REFUSALS = [
    "cannot create workflows",
    "can't create workflows",
    "cannot create or modify n8n workflows",
    "don't have a tool available that can create",
    "do not have a tool available that can create",
    "only n8n-related tool",
]


def response_text(response: dict[str, Any]) -> str:
    for key in ("reply", "message", "error", "raw"):
        value = response.get(key)
        if isinstance(value, str) and value.strip():
            return value
        if isinstance(value, dict):
            nested = response_text(value)
            if nested.strip():
                return nested
    return json.dumps(response, sort_keys=True)


def first_path_value(value: Any, paths: list[str]) -> Any:
    for path in paths:
        found = get_path(value, path)
        if found not in (None, ""):
            return found
    return None


def assert_chat_response(response: dict[str, Any], spec: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    text = response_text(response)
    lowered = text.lower()

    contains_any = _string_list(spec.get("expected_reply_contains_any", []))
    if contains_any and not any(item.lower() in lowered for item in contains_any):
        failures.append(
            "reply did not contain any expected text: " + ", ".join(contains_any)
        )

    contains_all = _string_list(spec.get("expected_reply_contains_all", []))
    missing_all = [item for item in contains_all if item.lower() not in lowered]
    if missing_all:
        failures.append("reply missing expected text: " + ", ".join(missing_all))

    contains_none = _string_list(spec.get("expected_reply_not_contains_any", []))
    unexpected = [item for item in contains_none if item.lower() in lowered]
    if unexpected:
        failures.append("reply contained forbidden text: " + ", ".join(unexpected))

    if spec.get("fail_on_generic_create_refusal"):
        matched = [item for item in GENERIC_CREATE_REFUSALS if item in lowered]
        if matched:
            failures.append("reply used generic n8n create/modify refusal")

    expected_status_any = _string_list(spec.get("expected_status_any", []))
    if expected_status_any:
        status = str(response.get("status") or "")
        if status not in expected_status_any:
            failures.append(
                f"status '{status or '<missing>'}' was not one of {expected_status_any}"
            )

    expected_action = spec.get("expected_n8n_action")
    if isinstance(expected_action, str) and expected_action:
        action = first_path_value(response, ["n8n.action", "n8n.routing.status"])
        if action != expected_action:
            failures.append(f"n8n action/status '{action}' did not equal '{expected_action}'")

    expected_action_any = _string_list(spec.get("expected_n8n_action_any", []))
    if expected_action_any:
        action = first_path_value(response, ["n8n.action", "n8n.routing.status"])
        if action not in expected_action_any:
            failures.append(
                f"n8n action/status '{action}' was not one of {expected_action_any}"
            )

    forbidden_actions = _string_list(spec.get("forbidden_n8n_actions", []))
    if forbidden_actions:
        action = first_path_value(response, ["n8n.action", "n8n.routing.status"])
        if action in forbidden_actions:
            failures.append(f"n8n action/status '{action}' is forbidden")

    forbidden_status_any = _string_list(spec.get("forbidden_status_any", []))
    if forbidden_status_any:
        status = str(response.get("status") or "")
        if status in forbidden_status_any:
            failures.append(f"status '{status}' is forbidden")

    forbidden_reply = _string_list(spec.get("forbidden_reply_contains_any", []))
    unexpected_reply = [item for item in forbidden_reply if item.lower() in lowered]
    if unexpected_reply:
        failures.append("reply contained forbidden text: " + ", ".join(unexpected_reply))

    _assert_nested_text_any(
        failures,
        response,
        spec,
        "expected_blocker_contains_any",
        ["blockers", "n8n.routing.blockers", "n8n.result.blockers"],
        "blocker",
    )
    _assert_nested_text_any(
        failures,
        response,
        spec,
        "expected_next_action_contains_any",
        ["next_actions", "n8n.routing.next_actions", "n8n.result.next_actions"],
        "next action",
    )
    _assert_nested_text_any(
        failures,
        response,
        spec,
        "expected_missing_inputs_any",
        ["missing_inputs", "n8n.routing.missing_inputs", "n8n.result.missing_inputs"],
        "missing input",
    )

    for assertion in _object_list(spec.get("expected_json_paths", [])):
        path = str(assertion.get("path") or "")
        if not path:
            failures.append("expected_json_paths entry is missing path")
            continue
        found = get_path(response, path)
        if assertion.get("exists", True) and found is None:
            failures.append(f"expected JSON path '{path}' was missing")
            continue
        if "equals" in assertion and found != assertion["equals"]:
            failures.append(
                f"JSON path '{path}' value {found!r} did not equal {assertion['equals']!r}"
            )
        if "contains" in assertion and str(assertion["contains"]) not in str(found or ""):
            failures.append(
                f"JSON path '{path}' value did not contain {assertion['contains']!r}"
            )

    for path in _string_list(spec.get("forbidden_json_paths", [])):
        found = get_path(response, path)
        if found is not None:
            failures.append(f"forbidden JSON path '{path}' was present")

    return failures


def _assert_nested_text_any(
    failures: list[str],
    response: dict[str, Any],
    spec: dict[str, Any],
    spec_key: str,
    paths: list[str],
    label: str,
) -> None:
    expected = _string_list(spec.get(spec_key, []))
    if not expected:
        return
    haystack_parts = []
    for path in paths:
        value = get_path(response, path)
        if value is None:
            continue
        if isinstance(value, list):
            haystack_parts.extend(str(item) for item in value)
        else:
            haystack_parts.append(str(value))
    haystack = "\n".join(haystack_parts).lower()
    if not any(item.lower() in haystack for item in expected):
        failures.append(f"{label} did not contain any expected text: " + ", ".join(expected))


def _string_list(value: Any) -> list[str]:
    if isinstance(value, list):
        return [str(item) for item in value if str(item)]
    if isinstance(value, str) and value:
        return [value]
    return []


def _object_list(value: Any) -> list[dict[str, Any]]:
    if isinstance(value, list):
        return [item for item in value if isinstance(item, dict)]
    return []
