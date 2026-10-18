# Local patched beta

This branch preserves the source used for the local `oo7-local-beta 0.7.0beta.local3-1` package. It is separate from the narrow PAM lifetime fix proposed upstream and is not an upstream release.

The focused upstream contribution is [PR #608](https://github.com/linux-credentials/oo7/pull/608). The additional transport corrections here remain local; related upstream work is tracked in [PR #592](https://github.com/linux-credentials/oo7/pull/592).

Base: [`0.7.0.beta`](https://github.com/linux-credentials/oo7/tree/be5dd7740cd740064a1eb99e037fe550343b06a0), commit `be5dd7740cd740064a1eb99e037fe550343b06a0`.

Changes:

- Backport upstream [5283f65](https://github.com/linux-credentials/oo7/commit/5283f65d2ce3c1daee0c835ae27475f864adfb10), accepting unknown D-Bus MIME types as binary data.
- Complete session-open and password-change handoffs synchronously, retaining `auto_start` and optional PAM success-on-error behavior.
- Require a positive daemon acknowledgement, limit responses to 64 KiB, and apply a 5-second deadline to the asynchronous socket exchange. The credential child retains upstream `waitpid`/`EINTR` handling; there is no numeric-PID kill watchdog or guaranteed deadline for the entire PAM call.

`Cargo.lock` and dependency versions are unchanged. The package includes daemon, login helper, PAM module, CLI and Secret portal.

## Build and focused tests

```sh
OO7_DAEMON_LOGIN_PATH=/usr/lib/oo7-daemon-login cargo build --locked --release -p oo7-daemon -p oo7-pam -p oo7-cli
cargo build --locked --release -p oo7-portal
OO7_DAEMON_LOGIN_PATH=/usr/lib/oo7-daemon-login cargo test --locked -p oo7-pam --lib
cargo test --locked -p oo7 --lib secret::tests::content_type_serialization -- --exact
```

The focused tests and isolated Linux-PAM C-caller tests passed, including immediate process exit, password-change persistence, helper-first startup and transport errors. Private migration and portal-backend tests also passed. This does not claim real SDDM/root-to-user login or desktop portal-frontend acceptance. Source hashes identify the tested changes; build commands alone do not promise bit-for-bit binary reproduction.

## Tested source SHA-256

- `client/src/secret.rs`: `5bdba668c35688bf34b9bc9325bb0d6b11d571d2209a15f9af7504328276fce1`
- `pam/src/lib.rs`: `5d69436e8576cfc75734bd75b0bd20e608b3ffaed6e0dd79194d4fd2c200b5b2`
- `pam/src/socket.rs`: `8537b565658cfab2578369e98c84bec750780ef3b59d880fc44ba2789ef5849f`

This local package is maintained separately. Replacing it with a stable upstream package requires an explicit review that the required PAM behavior is fixed or revalidated.
