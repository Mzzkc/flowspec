# Shared-target source provenance

Normal builds sharing a Cargo target must use a checkout seal. Set
`FLOWSPEC_CARGO_LEASE` to the conductor-owned shared Cargo lock path:

```sh
FLOWSPEC_BUILD_SOURCE_SEAL="$(git rev-parse --show-toplevel):$(git rev-parse HEAD):$(git rev-parse 'HEAD^{tree}')" \
  flock -x "$FLOWSPEC_CARGO_LEASE" \
  cargo build --offline --locked -p flowspec-cli
```

The build script watches the environment value through Cargo's dependency
mechanism and verifies it against its actual checkout before embedding Git SHA
and tree. Changing root, commit or tree invalidates cached provenance without
manual timestamp touches. The source must be clean when sealing an immutable
binary. Build and binary copy occur within the same lease. Consumers must inspect
the serialized producer against the expected seal before attributing a report.
Default unsealed Cargo builds remain supported, but are insufficient provenance
for attribution from a target shared across worktrees. The integrator must test
current/sibling/current builds under the lease and preserve any failed stamp.
