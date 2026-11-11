---
name: maidan-credits
description: Pay for a Maidan workspace's usage past its free allowance with prepaid credits. Use when a call to Maidan is refused with HTTP 402 Payment Required, or an MCP tool answers an error saying the workspace is past its free allowance for a meter; or when asked to check the workspace's balance or top it up. With a wallet, the agent pays the exact USDC amount on Base itself; without one, it hands a person the payment details.
---

# Maidan credits

An open sign-up workspace on a hosted Maidan gets a free allowance each UTC
month. Past it, on a meter the instance prices, usage is charged to the
workspace's prepaid credits. When the balance cannot cover the charge, the
work is refused until someone tops up:

- over HTTP, `402 Payment Required`, a problem whose `detail` names the meter
  (`requests`, `mcp_calls`, `embedding_calls`, `deliveries`, `wasi_ms`);
- over MCP, a tool error (`isError: true`) with the same text, and
  `structuredContent.meter`.

`stored_bytes` is different: it is a hard cap that credits never lift. Past it,
delete what the workspace no longer needs, or ask the operator.

Nothing here moves money by itself. Asking for a top-up only tells you what to
pay and where.

## When you are refused

1. Call `get_credits` with your `workspace_id` (from `whoami`). It works past
   the cutoff. Read `enabled` and `methods`: if `enabled` is false this
   instance takes no payment, so tell the person and stop. Read
   `balance_micros` (USD micros; below zero means overage went unpaid) and
   `prices`, the cost per meter past the allowance.
2. Decide how much to add. A few dollars covers a lot of calls at the listed
   prices; do not ask for more than the person allowed you to spend.
3. Call `top_up_credits` with `workspace_id`, `usd` (whole dollars, 1 to
   10000) and `method`.
   - `usdc` (the default) answers `amount_micros`, `amount`, `address`,
     `chain`, `chain_id`, `usdc_contract`, an `eip681` wallet link and
     `expires_at` (24 hours).
   - `card` is a hosted checkout for a person. It is the next payment rail;
     until an instance configures it, the call says card top-ups are not
     configured here.
4. Pay, or hand it on:
   - **You have a wallet** (a signer you were given for this): send exactly
     `amount_micros` base units of USDC, on that chain, to that address,
     before `expires_at`. The amount carries an offset in its last digits
     that names your workspace, so a rounded amount is not matched to you.
     Use a plain ERC-20 `transfer` from the `usdc_contract`; no memo is
     needed.
   - **You have no wallet**: give the person the `eip681` link (most wallets
     open it) or the address, chain and exact amount, and say why: which
     meter, what it costs, and what you were doing. With `card`, give them
     the checkout link instead.
5. Wait. The deposit counts once it has its confirmations (a few blocks on
   Base, well under a minute) and the watcher has read it. Call `get_credits`
   again: the balance rises, the intent leaves the open list, and the cutoff
   on priced meters lifts. Then retry the call that was refused.

Do not open a new intent for every retry: at most 20 can be open per
workspace, and an unpaid one simply expires. Reuse the one you were given
until it expires.

If you sent the wrong amount, it is not lost: the operator sees every
transfer that matched no intent and can credit it by hand. Tell the person
the transaction hash.

## Over HTTP

The same flow without MCP: `GET /workspaces/{id}/credits`, then
`POST /workspaces/{id}/credits/top-ups` with `{"usd": 5}` (or
`{"usd": 5, "method": "card"}`). Both need `workspace:read` and stay
reachable past the cutoff.

## Tools

`whoami`, `get_credits`, `top_up_credits`
