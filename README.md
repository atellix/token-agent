<div id="top"></div>

<!-- PROJECT LOGO -->
<br />
<div align="center">
  <a href="https://github.com/atellix/token-agent">
    <img src="https://media.atellix.net/atellix_token_agent.png" alt="Logo" width="512" height="512"/>
  </a>
  <h3 align="center">Token Agent</h3>
</div>

<!-- ABOUT THIS PROGRAM -->
## About This Program

The Token Agent program manages recurring subscriptions (a.k.a. "rebilling") and delegation of tokens to specific accounts.

Note: This program requires the "userAgent" account for each user be registered to the SPL token as the delegate in order to facilitate the later transfer of tokens on the user's behalf. For this reason, we recommend that other programs leave this setting in place. This Token Agent program provides a way to re-delegate token allowances to any number of accounts via a "TokenAllowance".

Recurring Subscriptions Features:
* Automatic token swapping (pay in one token, merchant gets a different token swapped at current prices).
* No double-billing
* Max budget
* Dynamic payment amount
* Ability to update subscription parameters
* User able to cancel subscription(s) directly

Features:
* Recurring subscriptions
* Delegate allowances to any number of accounts

This contract also adds the ability to create "TokenAllowance" accounts. The allowances enable a specific number of tokens to be delegated to be transfer later by a specific 3rd-party user. This program also include more fined-grained access controls than the SPL Token program, including time-based access, and limiting the recipient to a pre-defined account.

### Built With

* Rust
* Javascript
* [Anchor](https://project-serum.github.io/anchor/getting-started/introduction.html)

<!-- LICENSE -->
## License

Distributed under the MIT License.
