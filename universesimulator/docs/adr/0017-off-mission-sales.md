# 0017. Off-mission sales: the issuer buys what no mission counted

- **Status:** Proposed
- **Date:** 2026-11-11
- **Deciders:** WarpZone, ddurieux (Discord, 2026-11-10)
- **Scope:** universesimulator, mission, economie, social, game server
- **Amends:** 0012 (off-mission deliveries had no payment)

## Context

A player who mines, crushes and shelves ore in a warehouse without a mission (or more than their missions
ask) gave the goods away: the crate became the warehouse's, the stock rose, the player got nothing. The
team decided (2026-11-10) that such goods are bought at the market price.

Money moves through two calls to economie (a debit of the treasury, a credit of the player), and economie
answers a movement whose `externalId` it already knows with 409 `DUPLICATE_EXTERNAL_ID`, not with the
movement made. A first version paid, then recorded the sale: a credit refused after the debit (a missing
role, a frozen account) lost the money for good, every replay stopping on the debit's 409.

## Decision

- **What is sold.** The mission service's goods move (`POST /api/internal/missions/goods-moves`) answers
  how much of a drop the player's missions counted (`counted`); a replayed move answers the same, and
  `null` when the warehouses could not be resolved. The game server, for a crate the player owns (mined by
  them, never taken from a warehouse, no mission's load), sells the rest; it sells nothing on a `null` or
  on an answer without `counted`.
- **The route.** `POST /api/internal/warehouse-sales` (role `usim:deliver`): an idempotency key (at most 100
  characters), the warehouse, the good type, the quantity and the player.
- **Recorded, then paid.** The simulator prices the sale at the issuer's market price
  (`price_index.unit_price`, the town hall's board; the resource's base price when there is none) and
  records it, not paid (`warehouse_sales`, `paid_at` null). It then debits the treasury and credits the
  player, each movement keyed on the sale (`<key>:debit`, `<key>:credit`) and a movement already made taken
  as done, marks the sale paid and notifies the player through social (`warehouse.sale`), once.
- **A replay resumes.** The same key answers the sale it made: paid, as it is; not paid, by paying what is
  left to pay at the price recorded. The same key with another player, warehouse, good or quantity is
  refused (`IDEMPOTENCY_CONFLICT`).
- **An empty treasury mints.** A debit refused for `INSUFFICIENT_FUNDS` mints one batch (the monetary policy
  of 0009) and tries once more.
- svc-universesimulator needs `economie:wallet:credit` and `social:notify:write` (kubernetes #56).

## Consequences

- Mining pays with or without a mission; a mission pays more (its reward) for what it asks.
- A crate taken off a warehouse's shelf (a transport's load, `carrier_id`) or kept for a mission is never
  sold: putting it back is no sale.
- The amount is rounded to the credit: a sale worth less than half a credit pays nothing and is recorded.
- The sale is not tied to a delivery: the simulator pays the quantity the game server states, up to the
  room the warehouse had.
  Amended 2026-11-19 (WarpZone, playtest 2026-11-18; ddurieux to confirm): it was "nor to the warehouse's
  capacity ... A warehouse that is full still buys". With no limit, the only brake was the price, which
  stops falling at a quarter of the base: in preprod M0001 held 8 367 kg of crushed ore for a capacity of
  5 000 and a target of 4 000, and still bought. A warehouse now buys what fitted under its capacity for
  the resource before the drop (`bought`); the rest (`refused`) stays on its shelf, not paid, and the
  notice says how much. The stock is read from inventory at the sale, not from the last tick. A resource
  without a capacity is bought whole.
  The game refuses such a crate before it is given away (DyingStar#517): the depot's stock screen tags
  the good, a shelf of that depot takes no crate of it from its owner, the storekeeper leaves it on the
  counter and says why. `GET /api/warehouses/{id}` names each stock's `goodType` for that. The cap at
  the sale stays the last word, for a crate shelved before the game knew.
- A sale recorded and never replayed stays unpaid: the game server retries, and the table shows what is
  owed (`paid_at IS NULL`).
- The economie wallet records the credit as a `deposit` whose reference names the sale
  (`sale:<village>:<resource>`), which the DataPad's bank reads.
