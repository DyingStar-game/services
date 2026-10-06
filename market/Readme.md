# Service market

Marché de DyingStar : **carnet d'ordres** continu (ordres de vente pour les biens fongibles
et les instances uniques, ordres d'achat pour les biens fongibles), **demandes/contrats**
(B2B) et **règlement des échanges**. Ce service ne gère **que les flux d'échange** — il ne
touche jamais au monde physique. Un échange transfère la propriété des biens (déléguée au
service [`inventory`](../inventory/)) et les crédits (délégués à [`economie`](../economie/)),
chaque étape étant idempotente et rejouable tant que le trade est `pending`.

Stack : Node 22, TypeScript, Express 4, drizzle-orm + PostgreSQL, JWT Keycloak validé via
JWKS (`jose`). Structure calquée sur [`economie`](../economie/).

## Lancer en local

```bash
cp .env.example .env          # ajuster DATABASE_URL, OIDC_ISSUER, INVENTORY/ECONOMY/SOCIAL
docker compose -f docker/docker-compose.yml --env-file .env up   # API (watch) + postgres
```

Ou avec un Postgres déjà disponible : `pnpm install && pnpm dev` (migrations appliquées au
démarrage). Autres scripts : `pnpm build` / `pnpm start`, `pnpm type-check`, `pnpm db:generate`.
Smoke test : `node scripts/smoke.mjs`.

Pour l'API interne sans Keycloak : `INTERNAL_DEV_BYPASS=true` + `X-Internal-Key`. Pour les
routes joueur sans Keycloak : `AUTH_DEV_BYPASS=true` + `X-Player-Id` / `X-Player-Name`.

## Variables d'environnement

| Variable | Rôle |
|---|---|
| `PORT`, `NODE_ENV`, `CORS_ORIGIN` | HTTP |
| `DATABASE_URL` | PostgreSQL (obligatoire) |
| `OIDC_ISSUER`, `OIDC_JWKS_URL`, `OIDC_AUDIENCE` | Validation des JWT Keycloak joueurs |
| `OIDC_SERVICE_AUDIENCE` | `aud` attendu des tokens de service (défaut `market-api`) |
| `INTERNAL_SERVICE_CLIENTS` | Clients Keycloak de service autorisés (`azp`, CSV) |
| `INTERNAL_API_KEY` / `INTERNAL_DEV_BYPASS` | Repli dev `X-Internal-Key` sur `/api/internal/*` |
| `AUTH_DEV_BYPASS` | Accepter `X-Player-Id` sans JWT (dev uniquement) |
| `INVENTORY_API_URL`, `INVENTORY_SERVICE_CLIENT_ID/SECRET`, `INVENTORY_INTERNAL_API_KEY` | Intégration inventaire (transferts de biens) |
| `ECONOMY_API_URL`, `ECONOMY_SERVICE_CLIENT_ID/SECRET`, `ECONOMY_INTERNAL_API_KEY` | Intégration économie (débit/credit crédits) |
| `SOCIAL_API_URL`, `SOCIAL_SERVICE_CLIENT_ID/SECRET`, `SOCIAL_INTERNAL_API_KEY` | Intégration sociale (ACL `market:trade` ; vide = désactivé) |
| `MARKET_ORDER_TTL_HOURS` | Durée de vie par défaut d'un ordre (0 = jamais) |
| `MARKET_DEFAULT_LIMIT` | Taille de page par défaut |

## Modèle

- **Catalogue** (`market_catalog`) : liste des `goodType` échangeables, poussée par le
  serveur de jeu (`PUT /api/internal/catalog/:goodType`). La **nature** (fongible/instance)
  y est autoritative.
- **Ordres** (`market_orders`) : carnet. `sell` (fongible ou instance), `buy` (fongible
  uniquement). Matching immédiat : un achat croise le meilleur `sell` (prix le plus bas), une
  vente croise le meilleur `buy` (prix le plus haut) dès que les prix se croisent ; le trade
  s'exécute au prix de l'ordre au repos.
- **Demandes** (`market_demands`) : un détenteur demande un bien à un prix max ; n'importe
  quel détenteur peut `fulfill` (prix ≤ max), ce qui crée un trade.
- **Trades** (`market_trades`) : échange exécuté. Le règlement déplace les biens
  (`inventory.transferStack` / `transferInstance`) puis l'argent (`economy` débit acheteur +
  crédit vendeur), avec les drapeaux idempotents `goodsMoved`/`moneyMoved`. En cas d'échec,
  le trade reste `pending` et peut être rejoué.

**Pas de transfert magique** : le marché ne déplace jamais de bien physique ni ne crée de
propriété ; il s'appuie sur `inventory` (propriété) et `economie` (crédits). La confirmation
physique d'une livraison reste du ressort du serveur de jeu.

## Endpoints

Spécification complète : [`openapi.yaml`](openapi.yaml). Erreurs : `{ "error": "CODE", "message": "...", "status": 4xx }`.

Langue : envoyez **`Accept-Language: fr`** (ou `en`, **défaut `en`**) — les messages d'erreur (codes et texte dynamique) sont rendus dans la langue demandée ; les messages de validation zod restent en anglais. Les identifiants techniques (codes, kinds, champs, `{params}`) ne sont jamais traduits.

### Joueur (`Authorization: Bearer <JWT Keycloak>`)
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/market/catalog` | Types de biens échangeables |
| GET | `/api/market/book?goodType=` | Profondeur du carnet (nb d'achats/ventes ouverts) |
| GET | `/api/market/orders?goodType=&side=&status=&limit=` | Carnet d'ordres |
| POST | `/api/market/orders` `{side, goodType, kind, quantity, price, instanceId?, corporationId?}` | Passer un ordre (matching immédiat) |
| GET | `/api/market/orders/:id` | Un ordre |
| POST | `/api/market/orders/:id/cancel` | Annuler un de ses ordres |
| GET | `/api/market/demands?goodType=&status=&limit=` | Demandes/contrats |
| POST | `/api/market/demands` `{goodType, kind, quantity, maxPrice, instanceId?, message?, corporationId?}` | Créer une demande |
| POST | `/api/market/demands/:id/fulfill` `{unitPrice, corporationId?}` | Satisfaire une demande (règle le trade) |
| POST | `/api/market/demands/:id/cancel` | Annuler une de ses demandes |
| GET | `/api/market/trades?status=&limit=` | Mes trades (acheteur ou vendeur) |

`corporationId` (optionnel) fait trader depuis la trésorerie d'une corporation dont
l'appelant détient l'action **`market:trade`** — accordée à tout membre (`defaultMember`),
décidée par [`social`](../social/Readme.md) via `POST /api/internal/authorize`
(`403 NOT_CORPORATION_MEMBER` sinon ; Social indisponible → `503` sauf `AUTH_DEV_BYPASS=true`).

### Interne — serveur de jeu (token Keycloak de service + rôle de capacité)
| Méthode | Route | Rôle requis | Description |
|---|---|---|---|
| GET | `/api/internal/catalog` | `market:read` | Tout le catalogue (y compris désactivé) |
| PUT | `/api/internal/catalog/:goodType` `{kind, unit?, displayName?, enabled?}` | `market:manage` | Créer/mettre à jour un type de bien |
| PATCH | `/api/internal/catalog/:goodType` `{enabled}` | `market:manage` | Activer/désactiver un type |
| GET | `/api/internal/orders` | `market:read` | Carnet (tous détenteurs) |
| GET | `/api/internal/demands` | `market:read` | Demandes (tous détenteurs) |
| GET | `/api/internal/trades` | `market:read` | Trades (tous détenteurs) |
| POST | `/api/internal/orders` `{side, goodType, kind, quantity, price, instanceId?, currency?, holderType, holderId, corporationId?}` | `market:manage` | Passer un ordre pour un détenteur explicite (joueur, PNJ, corporation) |
| POST | `/api/internal/orders/:id/cancel` `{holderType, holderId, corporationId?}` | `market:manage` | Annuler un ordre d'un détenteur |
| POST | `/api/internal/demands` `{goodType, kind, quantity, maxPrice, instanceId?, message?, holderType, holderId, corporationId?}` | `market:manage` | Créer une demande pour un détenteur explicite |
| POST | `/api/internal/demands/:id/fulfill` `{holderType, holderId, unitPrice, corporationId?}` | `market:manage` | Satisfaire une demande en tant que détenteur (règle le trade) |
| POST | `/api/internal/demands/:id/cancel` `{holderType, holderId, corporationId?}` | `market:manage` | Annuler une demande d'un détenteur |
| GET | `/api/internal/trades/:id` | `market:read` | Un trade |
| POST | `/api/internal/trades/:id/settle` | `market:settle` | Rejouer le règlement d'un trade `pending` |

Détenteurs réglables par `economie` : `player`, `npc`, `corporation` (les PNJ ont des portefeuilles à parité avec les joueurs). Les transferts de biens passent par `inventory`, qui accepte aussi `npc`. `system` reste lisible/poussé par le serveur de jeu mais n'est pas réglé en argent par le marché.

## Structure

```
src/
  index.ts          bootstrap Express, migrations, listen
  config/env.ts     variables d'environnement
  db/schema/        tables drizzle (catalog, orders, demands, trades)
  db/connection.ts  pool pg + drizzle ; db/migrate.ts applique ./drizzle
  middleware/       auth (JWT joueur / service-account + rôles), validate (zod), errorHandler
  routes/           market.routes.ts (joueur), internal.routes.ts (serveur de jeu), schemas zod
  services/         market.service.ts, catalog.service.ts, clients inventory/economy/social
```

Déploiement : `docker/Dockerfile` (image standalone, migrations au démarrage).

## Feuille de route

- [x] Catalogue de biens (nature fongible/instance) poussé par le serveur de jeu
- [x] Carnet d'ordres achat/vente avec matching immédiat au meilleur prix
- [x] Demandes/contrats avec satisfaction directe
- [x] Règlement biens (inventory) + argent (economie), idempotent et rejouable
- [x] Trading pour le compte d'une corporation (ACL `market:trade` via `social`)
- [ ] Escrow des ordres au repos (réserver les biens/crédits dès la pose)
- [ ] Historique de prix / OHLC par type de bien
- [ ] Frais de marché et commissions dynamiques
- [ ] Expiration automatique des ordres/demandes dépassés (balayage)
