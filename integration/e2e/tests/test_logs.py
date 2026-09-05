"""log query surfaces against a real ClickHouse (#1177).

these two queries are only ever wrong against a live ClickHouse: the unit tests
in ``rolter-control`` pin the generated SQL text, but nothing in the Rust suite
executes it. both defects this module covers shipped because the dashboard e2e
spec stubs the endpoints.

gating: like every module under ``tests/``, this needs the live compose stack
(``uv run pytest tests/test_logs.py`` brings it up, or set
``ROLTER_E2E_NO_MANAGE=1`` against a stack you already run). ClickHouse is part
of that stack, so no extra opt-in flag applies.
"""

from __future__ import annotations

import time
import uuid

import pytest

from rolter_e2e.bootstrap import register_fleet
from rolter_e2e.client import ControlClient, GatewayClient
from rolter_e2e.stack import GATEWAY_URL


def test_mcp_logs_list_without_a_cursor(admin: ControlClient) -> None:
    """the first page carries no cursor, so `cursor_ts` binds to ''.

    a strict `parseDateTime64BestEffort('')` is still evaluated by ClickHouse and
    aborts the query, which made every first page fail (#1177).
    """
    resp = admin.raw("GET", "/api/v1/mcp/logs?limit=5")
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert "data" in body, body
    assert isinstance(body["data"], list), body


def test_mcp_logs_round_trip_an_event_and_page_it(admin: ControlClient) -> None:
    """ingest one event, then read it back through the list and the cursor."""
    event_id = f"e2e-{uuid.uuid4().hex}"
    server = f"e2e-server-{uuid.uuid4().hex[:8]}"
    admin.raw(
        "POST",
        "/api/v1/mcp/events",
        json={
            "event_id": event_id,
            "server": server,
            "tool": "search",
            "transport": "streamable_http",
            "status": "success",
            "latency_ms": 12,
            "request_id": f"req-{uuid.uuid4().hex[:8]}",
        },
        expect=202,
    )

    rows = _poll(lambda: _list_mcp(admin, f"?server={server}&limit=10"))
    assert rows, "the ingested event never became visible in the log list"
    assert rows[0]["event_id"] == event_id, rows[0]

    # the cursor path parses a real timestamp, so it must keep working alongside
    # the empty-cursor short circuit
    cursor = f"{rows[0]['ts']}|{rows[0]['event_id']}"
    resp = admin.raw("GET", f"/api/v1/mcp/logs?server={server}&limit=10&cursor={cursor}")
    assert resp.status_code == 200, resp.text
    assert resp.json()["data"] == [], resp.text


def test_invocations_expose_unqualified_payload_columns(admin: ControlClient) -> None:
    """the payload columns must arrive as `request_payload`/`response_payload`.

    ClickHouse names an unaliased qualified column `payload.request_payload`, and
    the dashboard's log drawer reads the unqualified names, so it always fell
    back to "payload logging is off" (#1177).
    """
    fleet = register_fleet(admin, models=1)
    gw = GatewayClient(GATEWAY_URL, virtual_key=fleet.virtual_key)
    try:
        _wait_for_model(gw, fleet.models[0])
        assert gw.chat(fleet.models[0], "logging probe").status_code == 200
    finally:
        gw.close()

    rows = _poll(lambda: _invocations(admin, f"?model={fleet.models[0]}&limit=5"), timeout=45.0)
    if not rows:
        pytest.skip("no request log reached clickhouse in time; the pipeline is async")
    row = rows[0]
    assert "request_payload" in row, sorted(row)
    assert "response_payload" in row, sorted(row)
    assert not [key for key in row if key.startswith("payload.")], sorted(row)


def _list_mcp(admin: ControlClient, query: str) -> list[dict]:
    resp = admin.raw("GET", f"/api/v1/mcp/logs{query}")
    assert resp.status_code == 200, resp.text
    return resp.json()["data"]


def _invocations(admin: ControlClient, query: str) -> list[dict]:
    resp = admin.raw("GET", f"/api/v1/analytics/invocations{query}")
    assert resp.status_code == 200, resp.text
    return resp.json()["data"]


def _poll(fetch, timeout: float = 20.0) -> list[dict]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        rows = fetch()
        if rows:
            return rows
        time.sleep(1.0)
    return []


def _wait_for_model(gw: GatewayClient, model: str, timeout: float = 20.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if gw.chat(model, "warmup").status_code == 200:
            return
        time.sleep(1.0)
    raise AssertionError(f"model {model} never became routable")
