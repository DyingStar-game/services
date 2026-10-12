# 0009. Rewards escrowed from the federation treasury; minting

- **Status:** Proposed
- **Date:** 2026-10-11
- **Deciders:** David Durieux
- **Scope:** universesimulator, mission, economie

## Context

A mission created through the mission service's internal API paid credits created from
nothing. ARES must pay its missions from its treasury, and ARES is the entity allowed to issue
money (`economie` `POST /api/internal/politics/:id/mint`, guarded by `allowMinting` and
`mintCeiling`).

## Options considered

1. **Keep credits from nothing**. No monetary model.
2. **Simulator debits the treasury itself**, mission still creates credits. Double booking.
3. **Internal escrow paid by a political entity** in the mission service, reusing the existing
   player escrow (debit at creation, refund on cancel/expiry, claimed at settlement).

## Decision

We will add an optional `escrow: { payerType: 'politics', payerId }` to the mission service's
internal `POST /missions`, guarded by the new service role `mission:escrow:politics`. The
simulator publishes every leg with the federation as escrow payer. Monetary policy: when the
escrow fails for lack of funds, or when the treasury falls below `treasury_floor`, the
simulator mints `mint_batch` for the federation and records it in `mint_events`. Because mint
has no idempotency key, it is called only by the tick, under the single-writer lock (ADR 0010).

## Consequences

Money in circulation is traceable to ARES' minting. The import enables minting for ARES
(`allowMinting`, `mintCeiling`) and creates its wallet.
