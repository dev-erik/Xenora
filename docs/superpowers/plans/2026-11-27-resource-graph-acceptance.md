# Bounded resource dependency graph acceptance (S2.7)

The existing opt-in graph contract admits four logical queues and sixteen retained
receipts, with unchanged payload budgets. Completion means validating composition
of supported resource operations, not adding unbounded queues, arbitrary future
waits, console synchronization semantics or automatic guest-memory coherence.

## Acceptance

- [x] Exercise mixed copy, geometry, attachments, descriptor sampling and storage
  dependencies across the four native queues with independently derived outputs.
- [x] Validate retained exact versions, ancestor/descendant collection, fan-in/out,
  failed-ancestor publication rejection and generation recovery.
- [x] Audit existing bounded closure, incomparable-writer rejection, node/payload
  pressure and explicit guest synchronization coverage; fix concrete gaps.
- [x] Run focused native/guest tests and meaningful isolated source mutations;
  restore sources exactly before final validation.
- [x] Obtain two independent reviews; authors do not certify their own components.
- [x] Update interface/ABI contract, checkpoint, changelog and validation entrypoints.
- [x] Run all nine gates on restored sources.

Validation: `artifacts/validation-full.tN7aM4`, 1,013 workspace passes and 461 native
suite passes (overlapping suites), zero failures. Two independent reviewers accepted
the bounded scope and source-restoration evidence. The lead publishes a signed-off
commit to the existing fork branch after this checklist.

Final integrated Stage 2 acceptance remains a separate milestone. Record evidence
and uncovered limits explicitly; passing existing tests alone does not close S2.7.
