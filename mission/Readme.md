# Service mission

API des missions de DyingStar : missions **dynamiques** (générées) et **scénarisées** (scripts), objectifs **vérifiables** (livraison de matériaux, transport d'une ressource d'un point A vers un point B), cycle de vie joueur (acceptation, progression, complétion, abandon) et récompenses **économiques** ou **items**.

Stack : Node 22, TypeScript, Express 4, drizzle-orm + PostgreSQL, JWT Keycloak validé via JWKS (`jose`). Structure et conventions calquées sur les services [`social`](../social/) et [`economie`](../economie/).

## Lancer en local

```bash
cp .env.example .env          # ajuster DATABASE_URL, OIDC_ISSUER, INTERNAL_SERVICE_CLIENTS, ECONOMY_API_URL
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
curl localhost:3000/api/missions -H "X-Player-Id: 11111111-1111-4111-8111-111111111111" -H "X-Player-Name: alice"
```

Pour l'API interne sans Keycloak, mettre `INTERNAL_DEV_BYPASS=true` (ignoré en production, où le service refuse de démarrer si le flag est actif) et passer `X-Internal-Key: <INTERNAL_API_KEY>`. En production, `/api/internal/*` n'accepte **qu'un** token de service Keycloak (`client_credentials`, `azp` autorisé, `aud` = `mission-api`, rôle de capacité requis) :

```bash
TOKEN=$(curl -s -X POST "$OIDC_TOKEN_URL" -d grant_type=client_credentials \
  -d client_id=svc-game -d client_secret="$SVC_GAME_CLIENT_SECRET" | jq -r .access_token)
curl localhost:3000/api/internal/missions -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"title":"Livraison","category":"delivery","objectives":[{"type":"deliver_material","title":"Minerai","targetQuantity":10}]}'
```

## Variables d'environnement

| Variable | Rôle |
|---|---|
| `PORT`, `NODE_ENV`, `CORS_ORIGIN` | HTTP |
| `DATABASE_URL` | PostgreSQL (obligatoire) |
| `OIDC_ISSUER`, `OIDC_JWKS_URL`, `OIDC_AUDIENCE` | Validation des JWT Keycloak joueurs (`player_id` = claim `sub`) |
| `OIDC_TOKEN_URL` | Endpoint `client_credentials` (défaut `${OIDC_ISSUER}/protocol/openid-connect/token`) |
| `OIDC_SERVICE_AUDIENCE` | `aud` attendu des tokens de service sur `/api/internal/*` (défaut `mission-api`) |
| `INTERNAL_SERVICE_CLIENTS` | Clients Keycloak de service autorisés (`azp`, CSV) sur `/api/internal/*` |
| `INTERNAL_API_KEY` | Secret hérité de `X-Internal-Key` — **dev uniquement** (voir `INTERNAL_DEV_BYPASS`) |
| `INTERNAL_DEV_BYPASS` | Accepter `X-Internal-Key` sur `/api/internal/*` (ignoré en production) |
| `AUTH_DEV_BYPASS` | Accepter `X-Player-Id` (+ `X-Player-Name`, `X-Player-Roles`) sans JWT (dev uniquement) |
| `ECONOMY_API_URL` | Base URL du service economie (séquestre et paiement des récompenses) |
| `ECONOMY_SERVICE_CLIENT_ID` / `ECONOMY_SERVICE_CLIENT_SECRET` | Compte de service Keycloak de mission (`svc-mission`) utilisé pour débiter/créditer les joueurs |
| `ECONOMY_INTERNAL_API_KEY` | Repli dev : `X-Internal-Key` envoyé à economie si aucun secret de service n'est configuré |
| `SOCIAL_API_URL` | Base URL du service social (vérification d'appartenance à une corporation) ; vide = missions corporation désactivées |
| `SOCIAL_SERVICE_CLIENT_ID` / `SOCIAL_SERVICE_CLIENT_SECRET` | Compte de service Keycloak (`svc-mission`) pour l'API interne sociale |
| `SOCIAL_INTERNAL_API_KEY` | Repli dev : `X-Internal-Key` envoyé à social |
| `MISSION_REWARD_AUTO_SETTLE` | Payer automatiquement les récompenses à la complétion (défaut `true`) |
| `MISSION_DEFAULT_TTL_HOURS` | Durée de vie par défaut d'une mission en heures (0 = jamais) |

## Fonctionnement

**Une mission** possède un `kind` (`dynamic`, `scenario`, `player`), une `category` (`delivery`, `transport`, `generic`), un émetteur (`issuerType` : `system`, `corporation`, `city`, `player` + `issuerId`), une `visibility` (`public` | `corporation`), un statut (`available`, `active`, `completed`, `cancelled`, `expired`), une `reward` (`{ economic?: {currency, amount}, item?: {itemId, quantity} }`) et une liste d'**objectifs**.

**Un objectif** est vérifiable : `type` (`deliver_material`, `transport`, `visit`, `custom`), `targetQuantity`, `currentProgress`, un `order` et des données libres (`locationFrom`, `locationTo`, `payload`). Pour une mission `scenario`, un objectif reste verrouillé (`409 OBJECTIVE_LOCKED`) tant que les objectifs d'ordre inférieur ne sont pas complétés.

**Multijoueur** : une mission avec `maxAssignees > 1` accepte plusieurs assignés indépendants qui **partagent les objectifs** (progression commune). À la complétion, tous les participants actifs sont complétés d'un coup et se **répartissent la récompense économique à parts égales** ; le reste de la division va aux premiers arrivés (tri par `acceptedAt`). La part de chacun est figée sur l'assignation (`rewardAmount`), ce qui rend un rejeu de paiement déterministe. La récompense **item** est refusée en multijoueur (`400 ITEM_REWARD_NOT_SPLITABLE`).

**Missions créées par les joueurs** (`POST /api/missions`) : n'importe quel joueur peut sponsoriser une mission à récompense **économique**, **séquestrée** depuis son portefeuille à la création (`economie:wallet:debit`, `externalId = mission-escrow:<missionId>`). Le séquestre est remboursé au créateur si la mission est annulée ou expire (`mission-refund:<missionId>`) et libéré aux participants à la complétion. Une mission peut être `public` ou réservée à une **corporation** (`visibility: corporation`, appartenance vérifiée auprès de social, `social:corporation:read`).

**Récompense** : à la complétion, le service crédite chaque participant via l'API interne d'economie (`POST /api/internal/players/:id/wallet/credit`) avec `externalId = mission:<missionId>:<playerId>`, ce qui rend le paiement **idempotent** (jamais de double crédit). Si le paiement échoue, les assignations restent `completed` avec `rewardSettled=false` et peuvent être rejouées via `POST /api/internal/missions/:missionId/settle`. Les récompenses **items** sont stockées en base (`{itemId, quantity}`) et seront réglées par un futur service inventaire.

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
| GET | `/api/missions?status=&kind=&category=&issuerType=&issuerId=&visibility=&limit=` | Missions disponibles (statut `available` par défaut) |
| POST | `/api/missions` `{title, reward.economic, visibility?, corporationId?, maxAssignees?, objectives}` | Créer une mission sponsorisée par le joueur (récompense séquestrée) |
| GET | `/api/missions/:missionId` | Détail + objectifs + mon assignation |
| POST | `/api/missions/:missionId/accept` | Accepter (crée l'assignation ; vérifie l'appartenance corporation) |
| POST | `/api/missions/:missionId/abandon` | Abandonner l'assignation |
| POST | `/api/missions/:missionId/objectives/:objectiveId/progress` `{quantity}` | Reporter la progression d'un objectif |
| POST | `/api/missions/:missionId/complete` | Compléter (tous les participants) et déclencher la répartition de la récompense |
| GET | `/api/me/missions?status=&limit=` | Mes missions (assignations + mission) |

### Interne — serveur de jeu (token Keycloak de service + rôle de capacité)
| Méthode | Route | Rôle requis | Description |
|---|---|---|---|
| POST | `/api/internal/missions` | `mission:write` | Créer une mission (system/IA/scénario) |
| GET | `/api/internal/missions?status=&kind=&category=&issuerType=&issuerId=&limit=` | `mission:read` | Catalogue complet (tous statuts) |
| PATCH | `/api/internal/missions/:missionId` | `mission:write` | Modifier titre, description, récompense, capacité, expiration |
| POST | `/api/internal/missions/:missionId/cancel` | `mission:write` | Annuler la mission, ses assignations actives et **rembourser le séquestre** |
| POST | `/api/internal/missions/expire` | `mission:write` | Expirer les missions dépassées (job planifié) et rembourser leur séquestre |
| POST | `/api/internal/missions/:missionId/objectives/:objectiveId/progress` `{playerId, quantity}` | `mission:progress` | Progression **vérifiée** côté serveur de jeu |
| POST | `/api/internal/missions/:missionId/complete` `{playerId, force?, settle?}` | `mission:complete` | Compléter tous les participants (vérifie les objectifs ; `force` pour outrepasser) |
| POST | `/api/internal/missions/:missionId/settle` `{playerId}` | `mission:complete` | Rejouer les parts de récompense non réglées (idempotent) |
| GET | `/api/internal/players/:playerId/missions` | `mission:read` | Assignations d'un joueur (+ mission) |

**Auth de service** : le garde est monté sur le préfixe (`app.use('/api/internal', serviceAuth, …)`), donc toute route ajoutée ici reste protégée. Le token doit venir d'un des clients `INTERNAL_SERVICE_CLIENTS` (`azp`), viser `OIDC_SERVICE_AUDIENCE`, puis détenir le rôle de capacité de la route — sinon `403 SERVICE_FORBIDDEN`/`FORBIDDEN`. Un token joueur ne peut jamais avoir un `azp` de service : c'est la garantie de non-contournement.

### Configuration Keycloak requise
- Client de service `svc-game` (serveur de jeu) autorisé dans `INTERNAL_SERVICE_CLIENTS`, avec les rôles `mission:read`, `mission:write`, `mission:progress`, `mission:complete`.
- Compte de service **`svc-mission`** (celui de ce service) :
  - economie doit l'autoriser dans son `INTERNAL_SERVICE_CLIENTS` et lui accorder `economie:wallet:credit` **et `economie:wallet:debit`** (séquestre) ;
  - social doit l'autoriser et lui accorder `social:corporation:read` (missions corporation).
- Création des clients/secrets/rôles gérée côté Keycloak (realm `dyingstar`, clients `svc-*`), hors realm JSON.

## Structure

```
src/
  index.ts          bootstrap Express, migrations, listen
  config/env.ts     variables d'environnement
  db/schema/        tables drizzle (missions, mission_objectives, mission_assignments)
  db/connection.ts  pool pg + drizzle ; db/migrate.ts applique ./drizzle
  middleware/       auth (JWT joueur / service-account + rôles de capacité), validate (zod), errorHandler
  routes/           un routeur par ressource, schémas zod dans routes/schemas.ts
  services/         logique métier (missions, assignations, récompenses, clients economie/social)
```

Déploiement : `docker/Dockerfile` (image standalone, migrations au démarrage), workflows `.github/workflows/build-*-mission.yaml`.

## Feuille de route

Ce service est dédié à la partie missions du jeu ; les features ci-dessous sont traitées pas à pas.

### Modèle & cycle de vie
- [x] Mission avec objectif vérifiable (livraison de matériaux, transport A→B, visite, custom)
- [x] Cycle de vie joueur : accepter, progresser, compléter, abandonner
- [x] Missions scénarisées avec objectifs ordonnés (verrouillage séquentiel)
- [x] Récompense économique ou item
- [x] Missions multijoueurs : objectifs partagés et récompense répartie entre participants
- [ ] Branches/conditions dans les scénarios (échec, embranchements)

### Missions dynamiques
- [ ] Génération à partir des besoins en ressources des corporations IA / villes (via l'API economie)
- [ ] Missions de livraison avec vérification de la quantité livrée avant paiement
- [ ] Régénération/rotation périodique et expiration

### Missions joueurs
- [x] Création de missions par les joueurs (récompense économique séquestrée)
- [x] Visibilité publique ou réservée à une corporation (via l'API interne sociale)
- [ ] Missions privées (invitation de joueurs précis)

### Récompenses
- [x] Paiement économique via l'API interne economie (idempotent)
- [x] Séquestre des récompenses des missions joueurs (débit/remboursement)
- [ ] Récompenses items via un futur service inventaire
- [ ] Récompenses de réputation (via l'API interne sociale)
