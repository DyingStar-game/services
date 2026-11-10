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
- The sale is not tied to a delivery nor to the warehouse's capacity: the simulator pays the quantity the
  game server states. A warehouse that is full still buys.
- A sale recorded and never replayed stays unpaid: the game server retries, and the table shows what is
  owed (`paid_at IS NULL`).
- The economie wallet records the credit as a `deposit` whose reference names the sale
  (`sale:<village>:<resource>`), which the DataPad's bank reads.
