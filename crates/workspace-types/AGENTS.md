# Workspace types

Keep domain values in `workspace`, `repository`, and `plugin`; put wire
contracts under the optional `schemas` module. Domain values must not import
wire contracts. Preserve distinct domain and wire representations.

Use the private `string_enum` source from `types`; keep its invocation name for
architecture analysis. Preserve schema names and serialized field behavior.

A domain value shared with a public wire schema still needs an external
production consumer; the admission rule checks both uses.

Keep repository URL parsing, normalization, and transport classification in
`workspaces::repository`; carry their resolved facts in shared data values.
