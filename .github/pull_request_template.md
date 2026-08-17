## What changes and why

<!-- The diff already tells the "what". Write the WHY: which problem this solves. -->

## How to test

<!-- Steps to see the change working, or the test that covers it. -->

## Checklist

- [ ] `npm run build` passes (types + frontend)
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean
- [ ] `cargo fmt --all --check` is clean
- [ ] Behaviour changes come with a test that **fails without the fix**
- [ ] New i18n keys landed in **both** `pt-BR.json` and `en.json`
