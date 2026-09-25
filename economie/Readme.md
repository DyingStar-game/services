# Service economie

API économique de DyingStar : comptes joueurs et corporations, portefeuilles multi-devises, transactions et trésorerie de corporation (à terme : marché, conversion, régulation et analytique).
Stack : Node 22, TypeScript, Express 4, drizzle-orm + PostgreSQL, JWT Keycloak validé via JWKS (`jose`). Structure et conventions calquées sur le service [`social`](../social/).

## Lancer en local

```bash
cp .env.example .env          # ajuster INTERNAL_API_KEY, DATABASE_URL, OIDC_ISSUER
docker compose -f docker/docker-compose.yml --env-file .env up   # API (watch) + postgres
```

Ou avec un Postgres déjà disponible :

```bash
pnpm install
pnpm dev                      # tsx watch ; les migrations de ./drizzle sont appliquées au démarrage
```

Autres scripts : `pnpm build` / `pnpm start` (prod), `pnpm type-check`, `pnpm db:generate` (après modification de `src/db/schema/`), `pnpm db:studio`.

Sans Keycloak, mettre `AUTH_DEV_BYPASS=true` (ignoré en production) et passer `X-Player-Id: <uuid>` (+ `X-Player-Name`) à la place du bearer :

```bash
curl localhost:3000/api/me/wallet -H "X-Player-Id: 11111111-1111-4111-8111-111111111111" -H "X-Player-Name: alice"
```

## Variables d'environnement

| Variable | Rôle |
|---|---|
| `PORT`, `NODE_ENV`, `CORS_ORIGIN` | HTTP |
| `DATABASE_URL` | PostgreSQL (obligatoire) |
| `OIDC_ISSUER`, `OIDC_JWKS_URL`, `OIDC_AUDIENCE` | Validation des JWT Keycloak (`player_id` = claim `sub`) |
| `INTERNAL_API_KEY` | Secret attendu dans `X-Internal-Key` sur `/api/internal/*` |
| `AUTH_DEV_BYPASS` | Accepter `X-Player-Id` (+ `X-Player-Name`, `X-Player-Roles`) sans JWT (dev uniquement) |
| `ECONOMY_TRANSFER_TAX_BPS` | Taxe automatique sur les transferts joueurs (basis points ; 500 = 5 %), prélevée côté payeur |
| `ECONOMY_TRANSFER_TAX_CEILING` | Plafond absolu de la taxe par transfert (0 = aucun) |
| `ECONOMY_MIN_TRANSFER` / `ECONOMY_MAX_TRANSFER` | Bornes d'un transfert (0 = pas de plafond) |
| `ECONOMY_TAX_VAULT_UUID` | Compte système réservé qui encaisse les taxes automatiques |

Montants en **unités entières** (crédits). Une devise est une simple chaîne (`credits` = monnaie universelle) ; un porteur possède un compte par devise (`unique(holder_type, holder_id, currency)`).

## Endpoints

Spécification complète (schémas, codes d'erreur) : [`openapi.yaml`](openapi.yaml).

Erreurs : `{ "error": "CODE", "message": "...", "status": 4xx }`.

### Public
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/health` | Liveness |

### Joueur (`Authorization: Bearer <JWT Keycloak>`)
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/me/wallet` | Mes comptes (un par devise, `credits` créé au premier appel) |
| GET | `/api/me/wallet/transactions?limit=` | Mon historique (toutes devises, plus récent d'abord) |
| POST | `/api/transfers` `{toPlayerId, amount, memo?}` | Transfert direct (taxe automatique `ECONOMY_TRANSFER_TAX_BPS` à la charge de l'émetteur) |

### Corporations (`Authorization: Bearer <JWT Keycloak>`) — adhésion renseignée par le serveur via l'API interne
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/corporations/:corporationId/wallet` | Trésorerie (membre) |
| GET | `/api/corporations/:corporationId/wallet/transactions?limit=` | Journal de trésorerie (membre) |
| GET | `/api/corporations/:corporationId/members` | Membres et rôles (membre) |
| GET | `/api/corporations/:corporationId/report?from=&to=` | Bilan financier par devise et par type (leader/trésorier) |
| POST | `/api/corporations/:corporationId/donations` `{amount, memo?}` | Don d'un membre (respecte `allowDonations`, taxe interne `taxRateBps`) |

Rôles de trésorerie : `leader` > `treasurer` > `member`.

### Interne — serveur de jeu (`X-Internal-Key`)
| Méthode | Route | Description |
|---|---|---|
| PUT | `/api/internal/players/:playerId/wallet` | Créer le compte `credits` au login (idempotent) |
| GET | `/api/internal/players/:playerId/wallet` | Comptes du joueur |
| GET | `/api/internal/players/:playerId/wallet/transactions?limit=` | Historique du joueur |
| POST | `/api/internal/players/:playerId/wallet/credit` `{amount, currency?, reference?, externalId?, type?}` | Créditer (prime de mission, salaire…) |
| POST | `/api/internal/players/:playerId/wallet/debit` | Débiter (refusé si solde insuffisant) |
| PUT | `/api/internal/corporations/:corporationId/wallet` | Créer le compte `credits` de la corporation |
| GET | `/api/internal/corporations/:corporationId/wallet` | Comptes de la trésorerie |
| GET | `/api/internal/corporations/:corporationId/wallet/transactions?limit=` | Journal de trésorerie |
| POST | `/api/internal/corporations/:corporationId/wallet/credit` `/debit` | Mouvements de trésorerie |
| PUT | `/api/internal/corporations/:corporationId/members/:playerId` `{role}` | Ajouter/mettre à jour un membre (leader/trésorier/member) |
| DELETE | `/api/internal/corporations/:corporationId/members/:playerId` | Retirer un membre |
| GET | `/api/internal/corporations/:corporationId/members` | Membres de la trésorerie |
| GET | `/api/internal/corporations/:corporationId/settings` | Taxe interne et politique de dons |
| PUT | `/api/internal/corporations/:corporationId/settings` `{taxRateBps?, allowDonations?}` | Régler la taxe interne / les dons |

**Idempotence** : les crédits/débits et transferts acceptent `externalId` (clé unique) — le serveur de jeu peut rejouer une écriture sans double comptabilisation (`409 DUPLICATE_EXTERNAL_ID` si déjà enregistrée).

**Comptes système** : les taxes automatiques sont encaissées sur le compte réservé `system` (`ECONOMY_TAX_VAULT_UUID`), créé à la demande. Solde jamais négatif (`CHECK` en base + `409 INSUFFICIENT_FUNDS`).

## Structure

```
src/
  index.ts          bootstrap Express, migrations, listen
  config/env.ts     variables d'environnement
  db/schema/        tables drizzle (accounts, transactions, corporation_members, corporation_settings)
  db/connection.ts  pool pg + drizzle ; db/migrate.ts applique ./drizzle
  middleware/       auth (JWT / clé interne / rôles), validate (zod), errorHandler
  routes/           un routeur par ressource, schémas zod dans routes/schemas.ts
  services/         logique métier (une fonction exportée par cas d'usage)
```

Déploiement : `docker/Dockerfile` (image standalone, migrations au démarrage), workflows `.github/workflows/build-*-economie.yaml`.

## Feuille de route

Ce service est dédié à la partie économique du jeu ; les features ci-dessous sont traitées pas à pas, en privilégiant celles reliées au service social (corps de métiers, trésorerie de corporation).

### Monnaie & Comptes
- [x] Comptes joueurs et corporations distincts (un compte par devise)
- [x] Transferts directs entre joueurs avec taxe automatique
- [x] Historique détaillé des transactions (ledger, taxe/frais, `externalId`)
- [x] Coffre système encaissant les taxes automatiques
- [ ] Frais de conversion variables selon le système ou la faction locale (multi-devises + taux)
- [ ] Coffres verrouillés (crédits bloqués, retraits restreints)

### Économie Sociale & Corporations
- [x] Trésorerie de corporation (compte commun) et rôles leader/trésorier/membre
- [x] Dons des membres (politique `allowDonations`, taxe interne `taxRateBps`)
- [x] Bilan financier détaillé (revenus, dépenses, par devise et par type)
- [ ] Classement économique des corporations (richesse, stabilité, influence)
- [ ] Système de sponsoring ou mécénat entre corporations

### Régulation & Sécurité (à venir)
- [ ] Détection de fraude / farming abusif
- [ ] Blocage automatique des transactions suspectes
- [ ] Sanctions économiques (pénalités, taxes forcées)
- [ ] Réputation économique personnelle (fiabilité, dette, solvabilité)

### Analytique & Statistiques (à venir)
- [ ] Masse monétaire totale, inflation, volume des transactions
- [ ] Classement des joueurs / corporations les plus riches
- [ ] Tableaux de bord publics et internes

### API & Intégrations
- [x] API interne synchronisée avec les serveurs du jeu (transactions, gains, marchés)
- [~] API publique sécurisée (JWT Keycloak en place ; clés tierces à venir)
- [ ] Conversion de devises et marché (prix, historique, commissions dynamiques)
- [ ] Webhooks sur les variations économiques
- [ ] Intégration au Service Social : affichage du rang économique, badges financiers, trésorerie, salaires automatiques, primes de mission