# Choose useful coverage

Start with a verifiable success criterion. A test should fail for the regression
being repaired and pass for the intended behavior. Prioritize domain decisions,
state transitions, response transformations, and user interactions with failure
or retry paths.

Skip tests that mirror a constant, restate a type, or assert presentational markup
with no behavior. Reversible documentation edits need link/schema checks, not
application tests. Executable helpers need tests for process status, output,
timeouts, and cleanup because these determine whether verification is trustworthy.

Run the relevant test once after the final change. Broaden testing when a failure
or dependency change gives a concrete reason, not to accumulate green output.
