# Service inventory

Registre de **propriété** de DyingStar : qui possède quels biens. Les biens sont soit
**fongibles** (une pile : type + quantité, ex. 500 t de minerai), soit **uniques** (une
instance avec un UUID stable, ex. un camion précis). Ce service ne stocke **que** la
propriété et les réservations — il ne connaît pas la position physique et **ne déplace
jamais** un bien tout seul : le serveur de jeu (ou le service `market`/`mission`) confirme
qu'un échange a réellement eu lieu, puis la propriété est transférée.

Stack : Node 22, TypeScript, Express 4, drizzle-orm + PostgreSQL, JWT Keycloak validé via
JWKS (`jose`). Structure et conventions calquées sur [`economie`](../economie/).

## Lancer en local

```bash
cp .env.example .env          # ajuster DATABASE_URL, OIDC_ISSUER, INTERNAL_SERVICE_CLIENTS
docker compose -f docker/docker-compose.yml --env-file .env up   # API (watch) + postgres
```

Ou avec un Postgres déjà disponible :

```bash
pnpm install
pnpm dev                      # tsx watch ; les migrations de ./drizzle sont appliquées au démarrage
```

Autres scripts : `pnpm build` / `pnpm start` (prod), `pnpm type-check`, `pnpm db:generate`,
`pnpm db:studio`. Smoke test : `node scripts/smoke.mjs`.

Sans Keycloak, mettre `AUTH_DEV_BYPASS=true` (ignoré en production) et passer
`X-Player-Id` (+ `X-Player-Name`) à la place du bearer. Pour l'API interne sans Keycloak,
mettre `INTERNAL_DEV_BYPASS=true` et passer `X-Internal-Key: <INTERNAL_API_KEY>`.

## Variables d'environnement

| Variable | Rôle |
|---|---|
| `PORT`, `NODE_ENV`, `CORS_ORIGIN` | HTTP |
| `DATABASE_URL` | PostgreSQL (obligatoire) |
| `OIDC_ISSUER`, `OIDC_JWKS_URL`, `OIDC_AUDIENCE` | Validation des JWT Keycloak joueurs (`player_id` = claim `sub`) |
| `OIDC_SERVICE_AUDIENCE` | `aud` attendu des tokens de service sur `/api/internal/*` (défaut `inventory-api`) |
| `INTERNAL_SERVICE_CLIENTS` | Clients Keycloak de service autorisés (`azp`, CSV) sur `/api/internal/*` |
| `INTERNAL_API_KEY` | Secret hérité de `X-Internal-Key` — **dev uniquement** |
| `INTERNAL_DEV_BYPASS` | Accepter `X-Internal-Key` sur `/api/internal/*` (ignoré en production) |
| `AUTH_DEV_BYPASS` | Accepter `X-Player-Id` (+ `X-Player-Name`, `X-Player-Roles`) sans JWT (dev uniquement) |
| `SOCIAL_API_URL` | URL interne du service social (membres corporation/politique : inventaires corpo + permissions POI ; vide = désactivé) |
| `SOCIAL_SERVICE_CLIENT_ID` / `SOCIAL_SERVICE_CLIENT_SECRET` | Compte de service Keycloak pour appeler social |
| `SOCIAL_INTERNAL_API_KEY` | Repli dev : clé partagée envoyée en `X-Internal-Key` si aucun secret n'est défini |

## Modèle

- **Détenteur** (`holder`) : `player`, `npc`, `corporation`, `system`. Ids opaques (UUID) ;
  les joueurs/PNJ et les corporations appartiennent au service `social`, il n'y a pas de
  copie locale.
- **Nature du bien** : un `goodType` est **fongible** (pile) ou **unique** (instance). Sa
  nature est enregistrée à la première utilisation et ne peut plus changer.
- **Réservation** (`hold`) : réserve des biens **disponibles** d'un détenteur pour une
  opération en attente (commande de marché, escrow de mission). Un hold ne change pas la
  propriété : `available = quantity - held`. La consommation d'un hold (`consume`) transfère
  réellement la propriété ; sa libération (`release`) rend les biens disponibles.
- **POI** (`inventory_pois`) : point GPS `{x, y, z}` éventuellement entouré d'une zone
  (`radiusM`), rattaché à une `scene` (préfixe) et/ou un `system`, avec un propriétaire
  (`player`/`npc`/`corporation`/`political`/`system`) et une visibilité
  (`private`/`public`). `parentId` est un uuid d'objet persistence opaque (nullable,
  pas de FK). La table `inventory_poi_shares` ajoute des grants **lecture seule**
  (player/npc/corporation/political) ; le propriétaire reste le seul à écrire.
  Transfert de propriété : `POST /api/me/pois/:poiId/transfer` (ou API interne).
  Lecture : owned ∪ granted ∪ public, plus les lectures `inventory:read` internes.
- Aucune notion de position ni d'interaction avec `persistence`/`resourcesDynamic`
  (hors référence `parentId` opaque).

## Endpoints

Spécification complète : [`openapi.yaml`](openapi.yaml). Erreurs : `{ "error": "CODE", "message": "...", "status": 4xx }`.

Langue : envoyez **`Accept-Language: fr`** (ou `en`, **défaut `en`**) — les messages d'erreur (codes et texte dynamique) sont rendus dans la langue demandée ; les messages de validation zod restent en anglais. Les identifiants techniques (codes, kinds, champs, `{params}`) ne sont jamais traduits.

### Public
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/health` | Liveness |
| GET | `/openapi.yaml` | Document OpenAPI (aussi `/api/openapi.yaml`) |

### Joueur (`Authorization: Bearer <JWT Keycloak>`)
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/me/inventory` | Mon inventaire (piles avec held/available + instances) |
| GET | `/api/me/inventory/stacks/:goodType` | Une pile, avec held/available |
| GET | `/api/corporations/:corporationId/inventory` | Inventaire d'une corporation (membre, via social) |
| GET | `/api/corporations/:corporationId/inventory/stacks/:goodType` | Une pile de corporation (membre) |
| GET | `/api/me/pois` | Mes POI (owned, granted, public) |
| POST | `/api/me/pois` | Créer un POI (propre, ou pour une corpo/entité politique gérée) |
| GET | `/api/me/pois/:poiId` | Un POI avec ses shares |
| PATCH | `/api/me/pois/:poiId` | Éditer un POI géré (ownership inchangé) |
| DELETE | `/api/me/pois/:poiId` | Supprimer un POI géré (shares en cascade) |
| POST | `/api/me/pois/:poiId/shares` `{granteeType, granteeId}` | Grant **lecture seule** |
| DELETE | `/api/me/pois/:poiId/shares/:granteeType/:granteeId` | Révoquer un grant |
| POST | `/api/me/pois/:poiId/transfer` `{toType, toId}` | Transférer la propriété |
| GET | `/api/corporations/:corporationId/pois[/:poiId]` | POI de la corporation (membre) |
| GET | `/api/politics/:entityId/pois[/:poiId]` | POI de l'entité politique (membre) |

### Interne — serveur de jeu / market / mission (token Keycloak de service + rôle de capacité)
| Méthode | Route | Rôle requis | Description |
|---|---|---|---|
| GET | `/api/internal/holders/:holderType/:holderId` | `inventory:read` | Inventaire complet d'un détenteur |
| GET | `/api/internal/holders/:holderType/:holderId/stacks/:goodType` | `inventory:read` | Une pile (held/available) |
| POST | `/api/internal/holders/:holderType/:holderId/credit` `{goodType, quantity}` | `inventory:credit` | Octroyer des biens fongibles |
| POST | `/api/internal/holders/:holderType/:holderId/debit` `{goodType, quantity}` | `inventory:credit` | Consommer des biens disponibles |
| POST | `/api/internal/holders/:holderType/:holderId/instances` `{instanceId, goodType, metadata?}` | `inventory:credit` | Enregistrer une instance unique |
| PATCH | `/api/internal/holders/:holderType/:holderId/instances/:instanceId` `{status}` | `inventory:credit` | Statut physique (`stored`/`in_world`) |
| POST | `/api/internal/transfers` `{from, to, goodType, quantity}` | `inventory:transfer` | Transférer des biens disponibles |
| POST | `/api/internal/transfers/instance` `{from, to, instanceId}` | `inventory:transfer` | Transférer une instance |
| POST | `/api/internal/holders/:holderType/:holderId/holds` `{kind, goodType, quantity?, instanceId?, refType, refId?}` | `inventory:hold` | Réserver des biens |
| POST | `/api/internal/holds/:holdId/release` | `inventory:hold` | Libérer un hold |
| POST | `/api/internal/holds/:holdId/consume` `{to}` | `inventory:transfer` | Consommer un hold (transfert) |
| POST | `/api/internal/pois/resolve` `{ids}` | `inventory:read` | Résolution batch de géométrie POI (zones de mission) |
| GET | `/api/internal/pois/:poiId` | `inventory:read` | Un POI avec ses shares |
| GET | `/api/internal/holders/:holderType/:holderId/pois` | `inventory:read` | POI d'un détenteur (dont `political`) |
| POST | `/api/internal/pois` | `inventory:poi:manage` | Créer un POI pour n'importe quel propriétaire |
| PATCH | `/api/internal/pois/:poiId` | `inventory:poi:manage` | Éditer un POI |
| DELETE | `/api/internal/pois/:poiId` | `inventory:poi:manage` | Supprimer un POI (shares en cascade) |

**Auth de service** : le garde est monté sur le préfixe (`app.use('/api/internal', serviceAuth, …)`).
Un crédit/débit/transfert n'est appliqué que sur confirmation d'un appelant de confiance —
c'est la garantie « pas de transfert magique ».

### Permissions POI (échec = `403`)

| Action | Joueur | Corporation | Entité politique |
|---|---|---|---|
| Lister / lire | owned ∪ granted ∪ public (`/api/me/pois`) | membre via social (`/api/corporations/…`) | membre via social (`/api/politics/…`) |

Les deux lignes d'écriture passent par **`POST /api/internal/authorize`** (`social:authorize`, action
`inventory:poi:manage`) : inventory n'interprète plus `rank.permissions` / `office.permissions`.
Un refus mappe sur `403 NOT_CORPORATION_MEMBER` / `NOT_POLITICAL_MEMBER` (non-membre) ou
`403 ORG_PERMISSION_REQUIRED` (membre sans l'action) ; Social indisponible → `502/503` (fail closed).
| Créer `/api/me/pois` | soi-même (ou `owner` passé) | action `inventory:poi:manage` (CEO ou `manage_corporation`) | action `inventory:poi:manage` (chef ou `manage_entity`) |
| Éditer / supprimer / partager / transférer | propriétaire `player` | propriétaire `corporation` | propriétaire `political` |
| Propriétaire `system` / `npc` | interdit (`403 NOT_POI_OWNER` / `POI_SYSTEM_OWNER_FORBIDDEN`) — API interne uniquement | | |

## Structure

```
src/
  index.ts          bootstrap Express, migrations, listen
  config/env.ts     variables d'environnement
  db/schema/        tables drizzle (goods, holds, holders, pois)
  db/connection.ts  pool pg + drizzle ; db/migrate.ts applique ./drizzle
  middleware/       auth (JWT joueur / service-account + rôles de capacité), validate (zod), errorHandler
  routes/           un routeur par ressource (me, politics, corporations, internal),
                    schémas zod dans routes/schemas.ts
  services/         logique métier (inventory.service.ts, pois.service.ts) ;
                    social.client.ts (membres corpo/politique)
```

Déploiement : `docker/Dockerfile` (image standalone, migrations au démarrage).

## Feuille de route

- [x] Piles fongibles (type + quantité) par détenteur
- [x] Instances uniques (UUID stable) rattachées à un détenteur
- [x] Réservations (holds) avec suivi `available = quantity - held`
- [x] Transferts directs et consommation de hold (règlement d'échange)
- [x] Autorisation des inventaires de corporation déléguée à `social`
- [x] Écriture des POI d'organisation déléguée à l'ACL centralisé (`inventory:poi:manage` via `POST /api/internal/authorize`)
- [x] POI (point/zone GPS, propriétaire, scènes, partage lecture seule, transfert)
- [ ] Expiration automatique des holds dépassés (balayage)
- [ ] Sous-entrepôts par rang/permission de corporation
- [ ] Intégration marché (offres/ordres) et récompenses item de mission
