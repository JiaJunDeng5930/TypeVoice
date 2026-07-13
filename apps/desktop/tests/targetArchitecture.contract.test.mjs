import assert from "node:assert/strict";
import test from "node:test";

import { textFromTranscriptionPartial } from "../src/domain/overlaySession.ts";
import { buildDiagnostic } from "../src/domain/diagnostic.ts";
import {
  shouldAcceptWorkflowProjection,
  workflowProjectionRevision,
  workflowViewFromPayload,
} from "../src/domain/workflowView.ts";

test("target_contract_t16_late_partial_is_scoped_to_active_run", () => {
  const activeRunId = "run-current";
  const latePartial = {
    kind: "transcription.partial",
    taskId: "run-old",
    message: "late partial",
    payload: { text: "must not be displayed" },
    tsMs: 10,
  };

  const displayed = textFromTranscriptionPartial(latePartial, activeRunId);

  assert.equal(
    displayed,
    "",
    `Target I5 violation: partial for ${latePartial.taskId} was displayed while ${activeRunId} is active; the production projection helper has no run identity boundary`,
  );
});

test("target_contract_t16_matching_partial_remains_visible", () => {
  const activeRunId = "run-current";
  const matchingPartial = {
    kind: "transcription.partial",
    taskId: "run-current",
    message: "current partial",
    payload: { text: "visible text" },
    tsMs: 11,
  };

  assert.equal(
    textFromTranscriptionPartial(matchingPartial, activeRunId),
    "visible text",
    "the run identity boundary must not discard the active run's own partial",
  );
});

function targetSnapshot(revision, overrides = {}) {
  return {
    mode: "ready",
    revision,
    actionKey: "Start(Initial)",
    activeRun: null,
    lastRun: null,
    primaryLabel: "START",
    primaryDisabled: false,
    cancelEnabled: false,
    ...overrides,
  };
}

test("target_contract_t19_latest_none_accepts_revision_zero", () => {
  const parsed = workflowViewFromPayload(targetSnapshot(0));
  const accepted = shouldAcceptWorkflowProjection(null, parsed);

  assert.ok(parsed, "target projection payload must be accepted");
  assert.equal(accepted, true, "latestRevision=None must accept the first valid projection");
  assert.equal(
    workflowProjectionRevision(parsed),
    0,
    `Unsupported target capability: frontend projection discards revision 0/actionKey; parsed=${JSON.stringify(parsed)}`,
  );
  assert.equal(parsed.actionKey, "Start(Initial)", "the opaque Primary actionKey must survive parsing");
});

test("target_contract_t19_listener_event_before_snapshot_keeps_new_revision", () => {
  let latestRevision = null;
  const listenerEvent = workflowViewFromPayload(targetSnapshot(2, {
    mode: "processing",
    actionKey: "Cancel(run-19)",
    activeRun: { runId: "run-19" },
  }));
  assert.ok(listenerEvent, "listener event must parse");
  if (shouldAcceptWorkflowProjection(latestRevision, listenerEvent)) {
    latestRevision = workflowProjectionRevision(listenerEvent);
  }

  const olderSnapshot = workflowViewFromPayload(targetSnapshot(1));
  assert.ok(olderSnapshot, "reconnect snapshot must parse");
  const acceptedOlderSnapshot = shouldAcceptWorkflowProjection(latestRevision, olderSnapshot);

  assert.equal(
    acceptedOlderSnapshot,
    false,
    `Target I11 violation: a listener event at rev2 followed by snapshot rev1 must retain rev2; latest=${latestRevision}, listener=${JSON.stringify(listenerEvent)}, snapshot=${JSON.stringify(olderSnapshot)}`,
  );
});

test("target_contract_t19_newer_projection_advances_revision", () => {
  const newer = workflowViewFromPayload(targetSnapshot(4));
  assert.ok(newer, "newer projection must parse");
  assert.equal(
    shouldAcceptWorkflowProjection(3, newer),
    true,
    "revision 4 must be accepted after revision 3",
  );
  assert.equal(workflowProjectionRevision(newer), 4, "accepted projection must expose revision 4");
});

test("target_contract_t19_reconnect_snapshot_cannot_roll_back", () => {
  const staleReconnect = workflowViewFromPayload(targetSnapshot(7));
  assert.ok(staleReconnect, "stale reconnect snapshot must parse before ordering is applied");
  assert.equal(
    shouldAcceptWorkflowProjection(8, staleReconnect),
    false,
    "a reconnect snapshot at revision 7 must not replace revision 8",
  );
});

test("target_contract_t19_equal_command_reply_keeps_view_but_processes_disposition", () => {
  const replyPayload = {
    disposition: "noOp",
    view: targetSnapshot(9, { actionKey: "Stop(run-19)" }),
  };
  const parsedReply = workflowViewFromPayload(replyPayload);
  assert.ok(parsedReply, "command reply boundary must parse a typed reply envelope");
  assert.equal(
    parsedReply.disposition,
    "noOp",
    `command disposition must survive independently of view replacement; parsed=${JSON.stringify(parsedReply)}`,
  );
  assert.equal(
    shouldAcceptWorkflowProjection(9, parsedReply.view),
    false,
    "an equal-revision command reply must not replace the current view",
  );
});

for (const [label, payload] of [
  ["unknown_mode", [
    targetSnapshot(1, { mode: "surprise" }),
    targetSnapshot(1, { legacyEffect: "stateChanging" }),
    {
      disposition: "noOp",
      view: targetSnapshot(1),
      legacyEventId: "removed-protocol-field",
    },
  ]],
  ["missing_revision", { mode: "ready", actionKey: "Start(Initial)" }],
  ["missing_action_key", { mode: "ready", revision: 1 }],
]) {
  test(`target_contract_t19_${label}_fails_closed`, () => {
    for (const candidate of Array.isArray(payload) ? payload : [payload]) {
      assert.equal(
        workflowViewFromPayload(candidate),
        null,
        `malformed projection ${label} must fail closed instead of becoming a startable Idle view`,
      );
    }
    if (label === "unknown_mode") {
      const diagnostic = buildDiagnostic(
        {
          code: "E_WORKFLOW_INTENT_INVALID",
          message: "workflow intent does not match the command schema",
        },
        "Workflow command failed",
      );
      assert.equal(
        diagnostic.code,
        "E_WORKFLOW_INTENT_INVALID",
        "typed workflow errors must preserve their structured code without string parsing",
      );
    }
  });
}
