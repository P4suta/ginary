<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# `.app` fixtures

Two kinds of file live here, and they are tested for different reasons.

## Hand-written fixtures

These exist to pin one behaviour each. They are small on purpose, so that a
failing assertion points at a single construct.

| file | what it pins |
|---|---|
| `quoted.app` | quoted atoms as the application name, as module and registered names, as an `env` key and inside `{mod, {'x@y', []}}`; `\"` and `\\` string escapes |
| `comments.app` | `%` and `%%` comments before, inside and after the term, and a `%` inside a string that is *not* a comment |
| `included.app` | `included_applications` alongside `applications` |
| `nested.app` | `env` holding binaries (`<<"...">>` and `<<>>`), character literals, negative and zero integers, floats including `-2.0e3`, nested lists and tuples, and `{}` |
| `malformed.app` | a missing `}`; the error must report line 5, column 3 |
| `unsupported_map.app` | a map literal; the error must name the construct and report line 7, column 21 |

`malformed.app` and `unsupported_map.app` are deliberately invalid. Nothing else
in this directory is.

## Copied fixtures

Real files, copied verbatim, because a parser that only handles files written by
its own author is not a parser. Do not reformat them: their whitespace,
comments and indentation are the point.

`otp/` — from the host Erlang/OTP 29.0.5 installation managed by mise, at
`~/.local/share/mise/installs/erlang/29.0.5/lib/<app>-<vsn>/ebin/<app>.app`:

| file | source directory |
|---|---|
| `otp/kernel.app` | `kernel-11.0.3` |
| `otp/stdlib.app` | `stdlib-8.0.3` |
| `otp/ssl.app` | `ssl-11.7.4` |
| `otp/inets.app` | `inets-9.7.1` |
| `otp/crypto.app` | `crypto-5.9.2` |

`shipment/` — from `gleam export erlang-shipment` run on the `notify` project
(`/home/<user>/projects/gleam/notify`, gleam 1.18.1, OTP 29.0.5) at
`build/erlang-shipment/<app>/ebin/<app>.app`:

| file | version at copy time |
|---|---|
| `shipment/notify.app` | 0.1.0 |
| `shipment/gleam_crypto.app` | 1.6.0 |
| `shipment/mist.app` | 6.0.3 |
| `shipment/gleam_stdlib.app` | 1.0.5 |

The two sets differ in ways that matter: OTP files are hand-written Erlang with
copyright headers and irregular indentation, while Gleam emits a uniform layout
and never emits `env`. Both shapes have to parse.

The host-wide check is separate: `parses_every_app_in_host_otp` in
`tests/appfile.rs` walks every `.app` under the *live* OTP root and asserts each
one parses. It is toolchain-gated, so it covers a machine with Erlang installed
and skips elsewhere; the copies above are what keeps the coverage on a machine
without one.

## Licensing

The hand-written fixtures are ginary's, under `MIT OR Apache-2.0` like the rest of the tree. The
copied ones are not: `otp/` is Ericsson's under `Apache-2.0` and `shipment/` belongs to the Gleam
packages it was generated from, also `Apache-2.0`. Neither set carries an SPDX header, because a
copy that has been edited is no longer a copy. `REUSE.toml` declares both paths instead.
