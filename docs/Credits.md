# Credits

A hosted Maidan gives each workspace a free allowance a month
(`MAIDAN_FREE_ALLOWANCE`, [Operations](Operations.md#the-free-allowance)).
Credits are how a workspace keeps going past it: a prepaid balance in US
dollars, topped up by payment and drawn down by overage on the meters the
instance prices. An agent can pay for itself.

Credits are off unless the operator sets a receiving address. With no address
or no prices, the instance behaves exactly as it did before credits: past the
allowance an open sign-up workspace is cut off until the month turns.

## Two payment rails, one balance

| Rail | Method | Status | Who pays |
|---|---|---|---|
| USDC on Base | `usdc` | this release | an agent with a wallet, or a person with any wallet |
| Card, through Stripe Checkout | `card` | **next** | a person, on a hosted checkout page |

Both credit the same ledger. Every deposit row carries its `source`
(`usdc_base`, `stripe`, or `grant` for an operator's credit) and an
`external_ref` unique per source: `<tx_hash>:<log_index>` for a USDC transfer,
the Checkout Session id for a card payment. The database holds that pair
unique, so a log read twice, a webhook delivered twice, or two replicas
reading the same block never credit twice. The card rail needs no migration:
the columns for it (`source`, `external_ref`, and on intents `method`,
`checkout_session_id` and `checkout_url`) are in place.

Until the card rail ships, `"method": "card"` answers `409` with "card top-ups
are not configured on this instance", and `GET /workspaces/{id}/credits` lists
only `usdc` in `methods`.

USDC on Base was picked first because it is the cheapest rail there is: no
card processor, a fee of a fraction of a cent, and an agent holding a wallet
can settle its own bill with no person in the loop.

## The ledger

The balance is the sum of the workspace's ledger, in USD **micros**. USDC has
six decimals, so one base unit of USDC is one micro-dollar, and a transfer's
amount is already in the ledger's unit.

- **Deposit** (+): a payment matched to one of the workspace's top-up
  intents.
- **Grant** (+): credit an operator added by hand, with a note.
- **Charge** (−): overage past the allowance on a priced meter. A charge
  records its meter, month and the units it covers.

The ledger has no foreign key to the workspace: a money record outlives an
erase, and a forced restore keeps the balance.

## Top-up intents

`POST /workspaces/{id}/credits/top-ups` with `{"usd": 5}` (and optionally
`"method": "usdc"`) opens an intent and answers `201`:

```json
{
  "method": "usdc",
  "chain": "base",
  "chain_id": 8453,
  "address": "0x…",
  "usdc_contract": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
  "usd": 5,
  "amount_micros": 5004217,
  "amount": "5.004217",
  "eip681": "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913@8453/transfer?address=0x…&uint256=5004217",
  "checkout_url": null,
  "status": "open",
  "expires_at": "…"
}
```

`usd` is whole dollars from 1 to 10,000. The amount asked for is the dollars
plus a random offset of 1 to 9,999 micros that no other open intent on the
instance holds for the same chain. That offset is how a plain ERC-20
transfer, which carries no memo, is matched to its workspace: send exactly
`amount_micros` base units, or it is not matched. An intent is offered for
24 hours; its amount stays reserved, and a transfer of it is still matched,
for 24 hours after that, so a payment sent in the last minutes still lands.
A workspace may hold 20 open intents at once.

Opening an intent moves no money, so it needs only `workspace:read`: an agent
can ask to be paid for itself. `GET /workspaces/{id}/credits` answers the
balance, whether the instance takes payment (`enabled`) and by which
`methods`, the chain, address and USDC contract, the overage `prices`, the
open intents and the recent ledger. Both routes stay reachable past the
allowance cutoff, and both are limited to the caller's own workspace.

The USDC contracts are Circle's: Base mainnet
`0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`, Base Sepolia
`0x036CbD53842c5426634e7929541eC2318f3dCF7e`.

## The deposit watcher

With `MAIDAN_CREDITS_ADDRESS` set, every replica runs a background loop
(`credits`) that:

1. asks the JSON-RPC endpoint (`MAIDAN_CREDITS_RPC_URL`) for the head block
   (`eth_blockNumber`);
2. reads the USDC `Transfer` logs (topic0
   `0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef`)
   whose `to` (topic2) is the receiving address (`eth_getLogs`), from the
   block after its cursor to the head less `MAIDAN_CREDITS_CONFIRMATIONS`
   (default 5), in ranges of at most 1,000 blocks;
3. records each log: one matching an open intent's exact amount credits that
   workspace and closes the intent; one that matches nothing is kept as
   **unmatched** for the operator, never dropped;
4. moves its cursor (kept in the store, per chain and address) past the
   range.

A first run starts 1,000 blocks back. A failed read stops the tick and the
next starts from the same block; the store records each log once, so a rerun
credits nothing twice. The watcher only reads the chain: it holds no key and
never signs or sends a transaction.

When a deposit lands and leaves the balance above zero, the workspace's
cutoff this month on the priced meters is lifted (its crossings are
removed, so `refresh_cut_off` stops refusing it), and its admins are told
through the `credits_deposited` event and its notification. A deposit that
only pays part of a debt lifts nothing.

## Charging overage

`MAIDAN_OVERAGE_PRICES` sets a price per meter as `meter=micros/units`,
comma-separated:

```
MAIDAN_OVERAGE_PRICES=mcp_calls=1000000/100000,requests=500000/100000
```

is one dollar per 100,000 MCP calls and fifty cents per 100,000 requests. Both
numbers are whole and at least 1. A malformed value, an unknown or repeated
meter, or a price on `stored_bytes` refuses boot, as `MAIDAN_FREE_ALLOWANCE`
does.

Each usage-meter tick, an open sign-up workspace past the allowance on a
priced meter is charged for the units past the limit that no charge covers
yet this month. Each charge starts where the last one for that workspace,
meter and month ended, and the database holds that start unique, so a rerun
or a second replica charges nothing twice. Charges in pieces cost exactly what
one charge would: the cost of units `(a, b]` is the cost through `b` less the
cost through `a`, each rounded up to a whole micro.

A workspace is cut off only when its balance cannot cover a charge. The charge
is then recorded in full, with the part the balance did not cover as
`unpaid_micros`; the balance goes below zero, the crossing is marked as of
that moment, and the next deposit pays the debt first. Counts reach the
meters once a flush interval, so a workspace can run up to about one
interval's usage past what its balance covers.

`stored_bytes` stays a hard cap: it is a level, not a count, so there are no
units to charge for. Past it a workspace is refused until it frees space or
the operator raises the allowance, whatever its balance.

Only open sign-up workspaces are charged, since only they are cut off.

## Refusals: 402

Past the allowance the refusal is `402 Payment Required`, problem type
`https://maidan.world/problems/payment-required`, whose detail names the
meter and says how to top up: `POST /workspaces/{id}/credits/top-ups` (MCP
`top_up_credits`). A Top up button on the console's Usage card follows in a
later release. Over MCP the refusal is a tool error (`isError: true`) with the
same text and the meter in `structuredContent`, so the model reads it; over
A2A it is `PermissionDenied` with the same text.

## How an agent pays

The [`maidan-credits`](https://github.com/david-engelmann/maidan/tree/main/examples/claude-code/maidan-credits)
skill teaches it. In short:

1. A call is refused with 402 or a tool error naming a meter.
2. `get_credits` shows the balance, the prices and the methods on offer.
3. `top_up_credits` with `usd` (and `method`) answers what to pay.
4. With a wallet, the agent sends exactly that USDC amount on Base itself.
   Without one, it hands a person the `eip681` link (or, once the card rail
   ships, the `checkout_url`).
5. Once the transfer has its confirmations, the balance rises and the cutoff
   lifts; the agent retries.

## For the operator

| Variable | Default | |
|---|---|---|
| `MAIDAN_CREDITS_ADDRESS` | unset | the receiving address; unset, credits are off |
| `MAIDAN_CREDITS_CHAIN` | `base` | or `base-sepolia` |
| `MAIDAN_CREDITS_RPC_URL` | the chain's public endpoint | a provider's endpoint is steadier: the public one rate-limits `eth_getLogs` |
| `MAIDAN_CREDITS_CONFIRMATIONS` | `5` | blocks on top of a transfer before it counts |
| `MAIDAN_CREDITS_POLL_SECS` | `15` | how often the watcher polls |
| `MAIDAN_OVERAGE_PRICES` | unset | price per meter, `meter=micros/units` |

`GET /operator/credits/unmatched` (`operator:global`) lists the transfers that
matched no intent. Settle one by hand: credit the workspace that sent it with
`POST /operator/workspaces/{id}/credits/grants` and `{"usd_micros": …,
"note": "…"}` (the note is kept on the ledger row), or refund it from the
wallet.
