# Tested main-based desktop package

This branch tracks a deliberately reviewed upstream main snapshot, with a small
PAM patch set. It is not an upstream release and does not automatically follow
new commits or merge open pull requests.

The initial source pin is `79e0ba4b388faa2313ab38d35cbda2cc4372206b`, based on
upstream `901e7406aa42f8be3bb9f5533ede8c5c7fec693d`. The upstream MIME, startup
ordering and GetSessionByPID fixes are included through main, without separate
backports.

The remaining local changes are:

- PR #608: complete PAM request delivery before the calling PAM operation returns.
- Bound the complete asynchronous socket exchange to five seconds, require a
  positive acknowledgement, cap response allocations at 64 KiB, and avoid logging
  password lengths or daemon-controlled error payloads. PR #592 overlaps response
  handling; review its final merged behavior before dropping these local changes.
- Clear supplementary groups in the root credential-switch child before dropping
  GID/UID. The calling process and same-user delivery retain their credentials.

The five-second deadline covers the asynchronous exchange, not every blocking
helper or credential-switch operation. PAM remains optional/best effort, and a
system password change and keyring password change are not atomic.

`packaging/arch/PKGBUILD` pins the tested source commit and builds the daemon,
login helper, PAM module, CLI and portal together using Cargo.lock. The package
replaces the historical local beta package. It does not modify PAM configuration,
keyrings, system boot files or restart the running provider.

Host resource limits are separate from the upstream code: a finite user-manager
hard limit and an oo7-only service limit with MALLOC_ARENA_MAX=1 let the existing
mlockall implementation work without granting capabilities or removing sandbox
settings. These host-specific settings are not automatically installed by the
public package recipe.

For updates, review new main commits and relevant PRs, keep only still-needed
patches, run the upstream Rust/PAM checks and isolated keyring upgrade/rollback
checks, then advance the source pin. Verify the new package and memory-lock
behavior before installation. Keep the previous package for rollback and
activate through a normal password login when ready.
