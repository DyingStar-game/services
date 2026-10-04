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
| `INVENTORY_API_URL` | Base URL du service inventory (séquestre et octroi des récompenses item) ; vide = récompenses item désactivées |
| `INVENTORY_SERVICE_CLIENT_ID` / `INVENTORY_SERVICE_CLIENT_SECRET` | Compte de service Keycloak (`svc-mission`) pour l'API interne inventaire |
| `INVENTORY_INTERNAL_API_KEY` | Repli dev : `X-Internal-Key` envoyé à inventory |
| `INVENTORY_SYSTEM_HOLDER_ID` | Détenteur `system` servant de source aux récompenses item non séquestrées |
| `MISSION_REWARD_AUTO_SETTLE` | Payer automatiquement les récompenses à la complétion (défaut `true`) |
| `MISSION_DEFAULT_TTL_HOURS` | Durée de vie par défaut d'une mission en heures (0 = jamais) |

## Fonctionnement

**Pipeline** : toute mission traverse les mêmes 4 étapes — **SPEC** (validation par le **registre de kinds**) → **ACCEPT** (prérequis + visibilité + groupe) → **PROGRESS** (dispatch par kind d'évaluation) → **REWARD** (récompenses en composants). Ajouter une capacité = ajouter un fichier dans `src/kinds/` (1 entrée de registre, 0 migration, aucune règle en dur dans le pipeline). Catalogue servi par **`GET /api/missions/kinds`** (catégories, kinds + JSON Schema de leurs params, forme des récompenses) ; dry-run par **`POST /api/missions/validate`**.

**Une mission** possède un `kind` (`dynamic`, `scenario`, `player`), une `category` (`delivery, transport, generic, mining, farming, crafting, construction, trading, exploration, salvage, combat, reception` — enum élargie en une ligne de code), un émetteur (`issuerType` + `issuerId`), une `visibility` (`public` | `corporation`), un statut, une liste **`rewards[]`**, une liste **`prerequisites[]`** et des **objectifs**. Trois colonnes pilotent le partage : `groupId`, `groupClaimable` (limitée à **1 groupe**), `isEvent`. **La spec est immuable après création** (objectifs, prérequis, récompenses, capacité figés — seul `title`, `description`, `expiresAt`, `isEvent` reste patchable) : le séquestre est pris dessus à la création.

**Catégories contraignantes** : chaque kind (objectif **et** prérequis) déclare ses catégories autorisées (`categories` dans `src/kinds/`), exposées par `GET /api/missions/kinds` — un builder filtre ainsi les kinds selon la catégorie choisie. La validation à la création rejette une incohérence (`400 OBJECTIVE_NOT_IN_CATEGORY` / `PREREQ_NOT_IN_CATEGORY`). Échappatoires : les kinds `'all'` (`custom`, `manual`, `min_reputation`, `corporation_member`) et la catégorie **`generic`** (bypass total). Matrice (hors `'all'`) :

| kind | catégories |
|---|---|
| `deliver_material` | delivery, transport, mining, farming, crafting, trading, salvage, reception |
| `deliver_items` | delivery, trading, reception, mining, farming, salvage |
| `transport` | transport, delivery, trading, reception |
| `visit` | exploration, combat, construction, salvage, delivery, reception |
| `owns_items` (objectif) | mining, farming, crafting, construction, trading, salvage |
| `has_credits` (objectif) | trading, reception, delivery |
| `owns_items` (prérequis) | mining, farming, crafting, construction, trading, salvage, delivery |
| `has_credits` (prérequis) | trading, delivery, construction, reception, mining, farming, crafting |

**Kinds d'objectifs** (registre `src/kinds/objectives/`) — 3 modes d'évaluation :
- **`game`** — le serveur de jeu soumet la quantité (`reportProgress`) : `deliver_material`, `transport`, `visit`, `custom` ;
- **`service`** — le service mesure contre l'état des services via **`POST /:missionId/verify`** : `owns_items` (instantané inventaire, peut régresser), `has_credits` (instantané solde), `deliver_items` (transfère les biens disponibles vers `to` **au moment du verify** — livraison terminale, irréversible) ;
- **`issuer`** — le créateur confirme : `manual` (`POST /:missionId/objectives/:objectiveId/confirm`, réservé au créateur ; le jeu confirme via l'API interne).

Un push `reportProgress` sur un kind non-`game` → `409 OBJECTIVE_NOT_PUSHABLE`. Pour une mission `scenario`, un objectif reste verrouillé (`409 OBJECTIVE_LOCKED`) tant que les précédents ne sont pas faits.

**Prérequis** (registre `src/kinds/prerequisites/`) : évalués à l'acceptation (joueurs uniquement, PNJ exclus) dans l'ordre, échec au premier → **`403 PREREQ_FAILED`** avec kind + détail. Kinds v1 : `has_credits` `{currency, amount}`, `owns_items` `{itemId, quantity, scope}`, `min_reputation` `{min}`, `corporation_member` `{corporationId}`.

**Disponibilité** : la liste joueur ne retourne que les missions **avec au moins une place libre** — une mission prise et pleine **disparaît de la liste des missions dispo**. Statuts par défaut `available` + `active` ; les missions partagées à un groupe ne sont listées que pour ses membres.

**Zones** (`zones[]`, vide = **globale**) : la dispo peut être restreinte à la **position du joueur** — scopes mappés sur la présence social (`PUT/GET /api/internal/players/:id/presence`) :
- `{kind:'system', system}` — même système ;
- `{kind:'scene', system?, scene}` — **chemin hiérarchique** de scène (`tarsis_1/new-paris`), match par égalité ou **préfixe de segment** (`tarsis_1` accepte `tarsis_1/new-paris`, jamais `tarsis_10`) → couvre planète, ville, district, donjon… **Convention jeu : encoder la hiérarchie dans `location.scene`** ;
- `{kind:'area', system?, center{x,y,z}, radiusM}` — POI local (distance euclidienne, unités jeu = mètres supposés).

Filtrée au **listing** (SQL, joueur sans position = ne voit que les globales ; panne Social = idem, fail-safe) **et à l'acceptation** (`403 OUT_OF_ZONE`, échec ferme). Zéro coût Social pour les missions globales. `zones` est **modifiable via l'API interne uniquement** (le jeu re-zone un event en direct ; sans impact séquestre).

**Groupes & partage** (`groupId`) : toute mission peut être **partagée à un groupe** (`POST /api/missions/:missionId/share`) — visible/acceptable uniquement par ses membres, comptée comme prise par eux (`409 MISSION_HAS_ASSIGNEES` tant qu'il y a des assignés actifs). Les missions `groupClaimable` sont **réclamées par le premier groupe** qui les accepte (`403 NO_GROUP` / `GROUP_ALREADY_CLAIMED`). Vérification via `social:group:read`.

**Missions event** (`isEvent`) : déclarées par le jeu (interne) ou une corporation (CEO / `manage_corporation`, `POST /:missionId/event`) — cap **1000 assignés** (sinon 100), retirer le flag exige `maxAssignees` ≤ 100 (`400 EVENT_CAPACITY_REQUIRES_FLAG`).

**Multijoueur** : `maxAssignees` = capacité (`1` unique, `N` places limitées, 1000 en event), combinable avec le partage à un groupe (places comptées **au sein du groupe claimant**). Objectifs partagés ; à la complétion, tous les participants actifs sont complétés d'un coup et reçoivent un **part égal de chaque composant de récompense**, figé dans **`rewardShares`** (rejeu de règlement déterministe). Composant **instance unique** ou **item séquestré** refusé en multijoueur (`400 ITEM_REWARD_NOT_SPLITABLE`).

**Missions joueurs** (`POST /api/missions`) : `rewards[]` **séquestrés à la création** — le composant crédits est débité du portefeuille du créateur (`economie:wallet:debit`, `externalId = mission-escrow:<missionId>`), **chaque** composant item est réservé en hold (`escrowItemHoldIds[]`, un par item). Remboursé si annulation/expiration (`mission-refund:<missionId>`). `visibility: corporation` vérifiée via social.

**Règlement** : à la complétion, **revérification** des objectifs `service` (sauf `force`) puis paiement composant par composant : crédits idempotents via `externalId = mission:<missionId>:<playerId>` ; items = **claim atomique** dans `settledComponents` (`item:<itemId>`) *avant* le transfert (hold séquestré consommé, sinon faucet `system`) — un échec partiel se rejoue sans jamais double-payer. Rejouer via `POST /api/internal/missions/:missionId/settle`.

## Endpoints

Spécification complète (schémas, codes d'erreur) : [`openapi.yaml`](openapi.yaml).

Erreurs : `{ "error": "CODE", "message": "...", "status": 4xx }`.

Langue : envoyez **`Accept-Language: fr`** (ou `en`, **défaut `en`**) — les messages d'erreur (codes et texte dynamique) sont rendus dans la langue demandée ; les messages de validation zod restent en anglais. Les identifiants techniques (codes, kinds, champs, `{params}`) ne sont jamais traduits.

### Public
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/health` | Liveness |
| GET | `/openapi.yaml` | Document OpenAPI du service (aussi exposé sur `/api/openapi.yaml`) |

### Joueur (`Authorization: Bearer <JWT Keycloak>`)
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/missions/kinds` | Catalogue des kinds (catégories, objectifs, prérequis, forme des récompenses) pour les builders |
| POST | `/api/missions/validate` `{mode?, mission}` | Dry-run d'une spec (aucune persistance, aucun séquestre) |
| GET | `/api/missions?status=&kind=&category=&issuerType=&issuerId=&visibility=&groupClaimable=&isEvent=&limit=` | Missions **avec une place libre** (statuts `available` + `active` par défaut ; groupes partagés visibles membres uniquement ; **zones** : sans position connue, seules les missions globales sont listées) |
| POST | `/api/missions` `{title, rewards[], prerequisites?, zones?, visibility?, corporationId?, maxAssignees?, groupClaimable?, objectives[{type(kind), params?…}]}` | Créer une mission (récompenses **séquestrées** à la création) |
| GET | `/api/missions/:missionId` | Détail + objectifs + mon assignation (404 si partagée à un groupe dont je ne suis pas membre) |
| POST | `/api/missions/:missionId/accept` | Accepter : corp + **zone** (`403 OUT_OF_ZONE`) + **prérequis** (`403 PREREQ_FAILED`) + groupe (`NOT_GROUP_MEMBER`, `NO_GROUP`, `GROUP_ALREADY_CLAIMED`) |
| POST | `/api/missions/:missionId/verify` | Re-mesurer les objectifs `service` (effectue les livraisons `deliver_items`) |
| POST | `/api/missions/:missionId/objectives/:objectiveId/confirm` | Confirmer un objectif `manual` (**créateur** de la mission uniquement) |
| POST | `/api/missions/:missionId/share` `{groupId}` | Partager la mission à un groupe (créateur ou membre corp ; `409 MISSION_HAS_ASSIGNEES`) |
| DELETE | `/api/missions/:missionId/share` | Retirer le partage (mêmes règles) |
| POST | `/api/missions/:missionId/event` `{enabled, maxAssignees?}` | Déclarer/retirer le flag **event** (mission corporation, CEO ou `manage_corporation`) |
| POST | `/api/missions/:missionId/abandon` | Abandonner l'assignation |
| POST | `/api/missions/:missionId/objectives/:objectiveId/progress` `{quantity}` | Reporter la progression d'un objectif **`game`** uniquement (`409 OBJECTIVE_NOT_PUSHABLE` sinon) |
| POST | `/api/missions/:missionId/complete` | Compléter : **revérifie** les objectifs `service`, puis répartit chaque composant de récompense |
| GET | `/api/me/missions?status=&limit=` | Mes missions (assignations + mission) |

### Interne — serveur de jeu (token Keycloak de service + rôle de capacité)
| Méthode | Route | Rôle requis | Description |
|---|---|---|---|
| POST | `/api/internal/missions` | `mission:write` | Créer une mission (`rewards[]`, `prerequisites[]`, `zones[]`, kinds, `groupClaimable`, `isEvent`) |
| GET | `/api/internal/missions?status=&kind=&category=&issuerType=&issuerId=&groupClaimable=&isEvent=&limit=` | `mission:read` | Catalogue complet (tous statuts, sans filtre zone) |
| PATCH | `/api/internal/missions/:missionId` | `mission:write` | Modifier **titre, description, expiration, `isEvent`, `zones`** seulement (spec immuable ; zones sans impact séquestre) |
| POST | `/api/internal/missions/:missionId/share` `{groupId}` | `mission:write` | Partager la mission à un groupe (canal de confiance) |
| DELETE | `/api/internal/missions/:missionId/share` | `mission:write` | Retirer le partage |
| POST | `/api/internal/missions/:missionId/event` `{enabled, maxAssignees?}` | `mission:write` | Déclarer/retirer le flag **event** (pas de vérif corporation) |
| POST | `/api/internal/missions/:missionId/cancel` | `mission:write` | Annuler la mission, ses assignations actives et **rembourser le séquestre** |
| POST | `/api/internal/missions/expire` | `mission:write` | Expirer les missions dépassées (job planifié) et rembourser leur séquestre |
| POST | `/api/internal/missions/:missionId/assign` `{playerId, holderType?}` | `mission:write` | Assigner une mission à un détenteur (`player` ou `npc`) |
| POST | `/api/internal/missions/:missionId/objectives/:objectiveId/progress` `{playerId, quantity}` | `mission:progress` | Progression **vérifiée** côté serveur de jeu (kinds `game`) |
| POST | `/api/internal/missions/:missionId/objectives/:objectiveId/confirm` `{}` | `mission:progress` | Confirmer un objectif `manual` (mission créée par le jeu/corp) |
| POST | `/api/internal/missions/:missionId/verify` `{playerId}` | `mission:progress` | Re-mesurer les objectifs `service` d'un détenteur |
| POST | `/api/internal/missions/:missionId/complete` `{playerId, force?, settle?}` | `mission:complete` | Compléter tous les participants (revérifie ; `force` pour outrepasser) |
| POST | `/api/internal/missions/:missionId/settle` `{playerId}` | `mission:complete` | Rejouer le règlement des composants non réglés (idempotent) |
| GET | `/api/internal/players/:playerId/missions` | `mission:read` | Assignations d'un joueur (+ mission) |

**Auth de service** : le garde est monté sur le préfixe (`app.use('/api/internal', serviceAuth, …)`), donc toute route ajoutée ici reste protégée. Le token doit venir d'un des clients `INTERNAL_SERVICE_CLIENTS` (`azp`), viser `OIDC_SERVICE_AUDIENCE`, puis détenir le rôle de capacité de la route — sinon `403 SERVICE_FORBIDDEN`/`FORBIDDEN`. Un token joueur ne peut jamais avoir un `azp` de service : c'est la garantie de non-contournement.

### Configuration Keycloak requise
- Client de service `svc-game` (serveur de jeu) autorisé dans `INTERNAL_SERVICE_CLIENTS`, avec les rôles `mission:read`, `mission:write`, `mission:progress`, `mission:complete`.
- Compte de service **`svc-mission`** (celui de ce service) — rôles à accorder sur les clients cibles :
  - **economie** (`INTERNAL_SERVICE_CLIENTS`) : `economie:wallet:credit`, `economie:wallet:debit` (séquestre) **et `economie:wallet:read`** (prérequis/objet `has_credits`) ;
  - **inventory** : **`inventory:read`** (mesure `owns_items`/`deliver_items` + prérequis) — les rôles de hold/transfer/credit utilisés par le règlement doivent aussi être accordés ;
  - **social** : `social:corporation:read` (missions corporation), `social:group:read` (partage/claim de groupes) **et `social:profile:read`** (prérequis `min_reputation`).
- Création des clients/secrets/rôles gérée côté Keycloak (realm `dyingstar`, clients `svc-*`), hors realm JSON.

## Structure

```
src/
  index.ts          bootstrap Express, migrations, listen
  config/env.ts     variables d'environnement
  db/schema/        tables drizzle (missions, mission_objectives, mission_assignments)
  db/connection.ts  pool pg + drizzle ; db/migrate.ts applique ./drizzle
  kinds/            REGISTRE de kinds — objectives/ et prerequisites/ (1 fichier par kind) + catalogue
  middleware/       auth (JWT joueur / service-account + rôles de capacité), validate (zod), errorHandler
  routes/           un routeur par ressource, schémas zod dans routes/schemas.ts
  services/         logique métier : spec.service (validation unique), prerequisites,
                    missions, assignations, récompenses, clients economie/inventory/social
```

Déploiement : `docker/Dockerfile` (image standalone, migrations au démarrage), workflows `.github/workflows/build-*-mission.yaml`.

## Feuille de route

Ce service est dédié à la partie missions du jeu ; les features ci-dessous sont traitées pas à pas.

### Modèle & cycle de vie
- [x] Mission avec objectif vérifiable (livraison de matériaux, transport A→B, visite, custom)
- [x] Cycle de vie joueur : accepter, progresser, compléter, abandonner
- [x] Missions scénarisées avec objectifs ordonnés (verrouillage séquentiel)
- [x] Récompense économique ou item
- [x] Récompense item réglée via le service inventory (séquestre à la création pour les missions joueur)
- [x] Missions multijoueurs : objectifs partagés et récompense répartie entre participants
- [x] Capacité (`maxAssignees`) : mission unique (1), places limitées (N) ou event (jusqu'à 1000)
- [x] Les missions sans place libre disparaissent de la liste des missions dispo
- [x] Missions réservées à 1 groupe (premier groupe claimant) et partage de toute mission à un groupe
- [x] Missions event déclarées par le jeu ou une corporation (CEO / `manage_corporation`)
- [ ] Branches/conditions dans les scénarios (échec, embranchements)

### Système programmable (registre de kinds)
- [x] Registre de kinds d'objectifs (`game` / `service` / `issuer`) + `GET /api/missions/kinds` (découverte builders)
- [x] Création unifiée via `validateMissionSpec` + dry-run `POST /api/missions/validate`
- [x] Récompenses en **liste de composants** (≤ 1 crédits + N objets), séquestre multi-holds, règlement idempotent par composant
- [x] Prérequis d'acceptation (`has_credits`, `owns_items`, `min_reputation`, `corporation_member`) → `403 PREREQ_FAILED`
- [x] Objectifs d'état mesurés (`owns_items`, `has_credits`) + endpoint `verify` (revérifiés avant paiement)
- [x] Livraison vérifiée avant paiement : kind `deliver_items` (transfert au verify vers la cible)
- [x] Kind `manual` : confirmation par l'émetteur (contrat type Eco)
- [x] Catégories ouvertes (enum élargie : mining, farming, crafting, …)
- [x] Missions **zonées** : dispo par système / scène hiérarchique (`scene` en chemin) / area (rayon) — filtrées au listing et à l'acceptation (`403 OUT_OF_ZONE`) selon la présence social
- [x] Spec immuable après création (précédent FTB : le séquestre est pris dessus)
- [x] **Break propre** : ancienne forme `reward` retirée, serveur de jeu à adapter (corps `rewards[]`, `params`, verify/confirm)
- [ ] Composition booléenne des prérequis (all/any/none), caution du contractor, missions répétables, chaînes `nextMissionId` (phase 2)
- [ ] Kind `traded_items` (vérifié via market) — phase 2

### Missions dynamiques
- [ ] Génération à partir des besoins en ressources des corporations IA / villes (via l'API economie)
- [x] Missions de livraison avec vérification de la quantité livrée avant paiement (`deliver_items`)
- [ ] Régénération/rotation périodique et expiration

### Missions joueurs
- [x] Création de missions par les joueurs (récompense économique et/ou item séquestrée)
- [x] Visibilité publique ou réservée à une corporation (via l'API interne sociale)
- [~] Missions privées (invitation de joueurs précis) — le **partage à un groupe** (groupe temporaire Social) est fait ; l'invitation de joueurs nommés reste à venir

### Récompenses
- [x] Paiement économique via l'API interne economie (idempotent)
- [x] Séquestre des récompenses des missions joueurs (débit/remboursement ; hold item/release)
- [x] Récompenses items via le service inventory (hold/consume ou transfert depuis `system`)
- [ ] Récompenses de réputation (via l'API interne sociale)
