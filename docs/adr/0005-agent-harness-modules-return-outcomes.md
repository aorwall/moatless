# Agent harness modules return outcomes

Each Agent harness is a deep module that owns CLI Session start/resume, input
delivery, native output normalization, cancellation, and harness-error
classification. The Sandbox creates one harness instance, which may replace its
Agent process while preserving the Session. Harness modules return normalized
outcomes and do not receive Task lifecycle state or the backend event publisher;
the harness-neutral coordinator alone owns Run identity, Turn state,
acknowledgement, retry policy, and event publishing. Normalization preserves one
source event per native content block; Message assembly remains exclusively in
the backend under ADR-0002 and ADR-0003.
