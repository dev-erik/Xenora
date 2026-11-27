# Four-Queue Resource Graph Increment

**Goal:** Extend opt-in graph execution from two to four logical/native queues without changing node or payload ceilings.
**Architecture:** Preserve exact RenderPoint identities and independent event timelines per queue. All queues share the existing device; graph cohorts admit four, legacy cohorts admit only First/Second. The worker maps new logical queue handles only after successful native admission.
**Tech Stack:** Native ARM64 Metal, Rust, original synthetic guest tests.

## Scope and remaining S2.7

This is the first S2.7 increment, not completion of general dependency graphs.
Sixteen retained graph nodes and existing payload ceilings remain unchanged.
Queue assignment is retained for the current worker generation, including after
receipt collection; a fifth distinct queue rejects. Reinitialization clears the
assignment and gives every native queue a fresh event/timeline. Legacy cohorts
still allow two entries and only First/Second submissions. Queue accessors expose
all four physical queues; legacy admission rejects the extra lanes.

Follow-up S2.7 acceptance still needs a graph-wide matrix combining attachment,
copy, geometry, descriptor/storage and synchronization paths; wider fan-in/fan-out
failure/recovery and lifetime pressure; and an explicit final dependency contract.
Final integrated Stage2 acceptance remains separate. No arbitrary/unbounded queue
support or game compatibility is claimed.

## Steps

- [x] Add Third/Fourth native queues, event/sequence/latest ownership, legacy admission guard and four-lane reinitialization in render_cohort.rs.
- [x] Extend all five worker queue mappings, including attachment transactions, without consuming assignments on failure.
- [x] Independently test native chain/fan-out/fan-in, exact outputs and legacy rejection; test original guest four-queue chains, descendant-first collection, fifth-queue rejection and retry.
- [x] Independently review generation poisoning, draining and queue identity; test recovery for added queues and assert regression detection through isolated source faults.
- [x] Register focused tests, update contracts/changelog/checkpoint and run all nine repository gates with at most four Cargo jobs and one GPU workload.
- [x] Signed-off commit and push existing fork branch; verify remote hash and clean tree. Report S2.7 as still open.
