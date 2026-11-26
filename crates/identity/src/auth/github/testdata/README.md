# Test data for GitHub App tests

`github_app_test_key.pem` is a throwaway 2048-bit RSA key generated for this
repository's tests. It signs nothing real: no GitHub App has ever held it, and
the only code that reads it is `#[cfg(test)]` code in `crates/identity`.

It is committed rather than generated at test time because the tests need a key
that `jsonwebtoken`'s `EncodingKey::from_rsa_pem` accepts, and no RSA key
generator is in the dependency tree.

If it ever needs replacing:

```sh
openssl genrsa -traditional 2048 > github_app_test_key.pem
```
