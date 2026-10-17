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

Autres scripts : `pnpm build` / `pnpm start` (prod), `pnpm type-check`, `pnpm db:generate` (après modification de `src/db/schema/`), `pnpm db:studio`, `pnpm db:reset -- --yes` (vide le schéma pour repartir de zéro : la migration unique `0000_init` est rejouée au prochain démarrage).

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
| `INVENTORY_API_URL` | Base URL du service inventory (séquestre et octroi des récompenses item, **résolution des zones `poi`**) ; vide = récompenses item et zones POI désactivées |
| `INVENTORY_SERVICE_CLIENT_ID` / `INVENTORY_SERVICE_CLIENT_SECRET` | Compte de service Keycloak (`svc-mission`) pour l'API interne inventaire |
| `INVENTORY_INTERNAL_API_KEY` | Repli dev : `X-Internal-Key` envoyé à inventory |
| `INVENTORY_SYSTEM_HOLDER_ID` | Détenteur `system` servant de source aux récompenses item non séquestrées |
| `MISSION_REWARD_AUTO_SETTLE` | Payer automatiquement les récompenses à la complétion (défaut `true`) |
| `MISSION_DEFAULT_TTL_HOURS` | Durée de vie par défaut d'une mission **interne** en heures (0 = jamais) |
| `MISSION_PLAYER_DEFAULT_TTL_HOURS` | Durée de vie par défaut d'une mission **joueur** en heures (défaut `72`) — bornée par `expiresAt`/`expiresInHours` |
| `MISSION_MAX_ACTIVE_ASSIGNMENTS` | Assignations actives simultanées max par joueur (défaut `5`, 0 = illimité ; PNJ exclus) |
| `MISSION_EXPIRE_INTERVAL_SECONDS` | Intervalle du balayage d'expiration **intégré** en secondes — **`0` par défaut (désactivé)** : en Kubernetes l'expiration est une **commande** (CronJob appelant `POST /api/internal/missions/expire`), le service ne tourne aucun timer ; renseigner une valeur positive réactive le balayage in-process (dev monoposte) |

## Fonctionnement

**Pipeline** : toute mission traverse les mêmes 4 étapes — **SPEC** (validation par le **registre de kinds**) → **ACCEPT** (prérequis + visibilité + groupe) → **PROGRESS** (dispatch par kind d'évaluation) → **REWARD** (récompenses en composants). Ajouter une capacité = ajouter un fichier dans `src/kinds/` (1 entrée de registre, 0 migration, aucune règle en dur dans le pipeline). Catalogue servi par **`GET /api/missions/kinds`** (catégories, kinds + leur **`name`** bilingue localisé, leur `summary` et le JSON Schema de leurs params, forme des récompenses) ; dry-run par **`POST /api/missions/validate`**.

**Une mission** possède un `kind` (`dynamic`, `scenario`, `player`), une `category` (`delivery, transport, mining, exploration, divers` — enum élargie en une ligne de code), un émetteur (`issuerType` + `issuerId`), une `visibility` (`public` | `corporation`), un statut, une liste **`rewards[]`**, une liste **`prerequisites[]`** et des **objectifs**. Trois colonnes pilotent le partage : `groupId`, `groupClaimable` (limitée à **1 groupe**), `isEvent`. **La spec est immuable après création** (objectifs, prérequis, récompenses, capacité figés — seul `title`, `description`, `expiresAt`, `isEvent` reste patchable) : le séquestre est pris dessus à la création.

**Catégories contraignantes** : chaque kind (objectif **et** prérequis) déclare ses catégories autorisées (`categories` dans `src/kinds/`), exposées par `GET /api/missions/kinds` — un builder filtre ainsi les kinds selon la catégorie choisie. La validation à la création rejette une incohérence (`400 OBJECTIVE_NOT_IN_CATEGORY` / `PREREQ_NOT_IN_CATEGORY`). Échappatoires : les kinds `'all'` (`manual`, `min_reputation`, `min_standing`) et la catégorie **`divers`** (bypass total). Matrice (hors `'all'`) :

| kind | catégories |
|---|---|
| `deliver_material` | delivery, transport, mining |
| `transport` | transport, delivery, mining |
| `transport_passenger` | transport |
| `visit` | exploration, delivery, transport, mining |
| `owns_items` (objectif) | mining, delivery, transport |
| `owns_items` (prérequis) | mining, delivery, transport |

**Kinds d'objectifs** (registre `src/kinds/objectives/`) — 2 modes d'évaluation :
- **`service`** — le service mesure contre l'état du joueur (présence + biens portés) via **`POST /:missionId/verify`** : `visit` (présence dans le POI, latché), `deliver_material` (biens `carried` dans la zone de livraison, high-water mark), `transport` (latch `pickedUpAt` + livraison, high-water mark), `transport_passenger` (passager dans le POI, high-water mark), `owns_items` (instantané inventaire, peut régresser) ;
- **`issuer`** — le créateur confirme : `manual` (`POST /:missionId/objectives/:objectiveId/confirm`, réservé au créateur ; le jeu confirme via l'API interne).

**Sémantique high-water mark** : `visit`, `deliver_material`, `transport` et `transport_passenger` ne régressent jamais — `progress = max(stored, measured)`. Une livraison partielle reste créditée (`allowPartial: true` paie le ratio en crédits) et l'auto-complétion se déclenche à 100 %.

Un push `reportProgress` sur un kind non-`game` → `409 OBJECTIVE_NOT_PUSHABLE`. Pour une mission `scenario`, un objectif reste verrouillé (`409 OBJECTIVE_LOCKED`) tant que les précédents ne sont pas faits.

**Prérequis** (registre `src/kinds/prerequisites/`) : évalués à l'acceptation (joueurs uniquement, PNJ exclus) dans l'ordre, échec au premier → **`403 PREREQ_FAILED`** avec kind + détail. Kinds v1 : `min_standing` `{domain, min}` (domain = catégorie ou `general` — RMS dérivé), `min_reputation` `{min}`, `owns_items` `{itemId, quantity, scope}`.

**Disponibilité** : la liste joueur ne retourne que les missions **avec au moins une place libre** — une mission prise et pleine **disparaît de la liste des missions dispo**. Statuts par défaut `available` + `active` ; les missions partagées à un groupe ne sont listées que pour ses membres.

**Zones** (`zones[]`, vide = **globale**) : la dispo peut être restreinte à la **position du joueur** — scopes mappés sur le store d'état joueur (`PUT /api/internal/missions/player-states`, TTL 30 s). **Les positions sont relatives au corps nommé par `scene`** (repère local, mètres ; `scene: "space"` = espace ouvert, repère du système) :
- `{kind:'system', system}` — même système ;
- `{kind:'poi', poiId, radiusM?}` — **point ou zone du registre POI** d'`inventory` (résolution batch `POST /api/internal/pois/resolve`, cache 60 s) :
  - **La `scene` du joueur doit être compatible avec la `scene` du POI** avant toute comparaison de coordonnées (égalité ou préfixe de segment : `tarsis_1` ⟷ `tarsis_1/new-paris`, jamais `tarsis_1` ⟷ `tarsis_10`) — sans cette garde, un joueur sur `tarsis_3` matchait un POI sur `tarsis_4` à la même distance locale. Un POI sans `scene` ne matche jamais (échec fermé) ;
  - `radiusM` déclaré sur la **zone** (prioritaire, **borné par le rayon de la POI**) ou sur le **POI** → **aire** : distance euclidienne au centre dans le repère partagé ;
  - POI avec `scene` et sans rayon → **sous-arbre de scène** (le joueur doit être dans la scène du POI ou en dessous) → couvre planète, ville, district, donjon… **Convention jeu : encoder la hiérarchie dans `location.scene`**.

  Un `system` porté par le POI doit être égal à `location.system`. Un POI inconnu/supprimé → la zone **ne matche jamais** (échec ferme) ; le registre refuse la création d'une zone sur un POI inexistant (`400 POI_NOT_FOUND`) ou avec un rayon supérieur à celui de la POI (`400 POI_RADIUS_EXCEEDED`).

Filtrée au **listing** (SQL, géométries POI injectées en jsonb ; joueur sans position = ne voit que les globales ; panne Social **ou** inventory = idem, fail-safe) **et à l'acceptation** (`403 OUT_OF_ZONE`, échec ferme). Zéro coût Social/inventory pour les missions globales. `zones` est **modifiable via l'API interne uniquement** (le jeu re-zone un event en direct ; sans impact séquestre).

**Groupes & partage** (`groupId`) : toute mission peut être **partagée à un groupe** (`POST /api/missions/:missionId/share`) — visible/acceptable uniquement par ses membres, comptée comme prise par eux (`409 MISSION_HAS_ASSIGNEES` tant qu'il y a des assignés actifs). Les missions `groupClaimable` sont **réclamées par le premier groupe** qui les accepte (`403 NO_GROUP` / `GROUP_ALREADY_CLAIMED`). Vérification via `social:group:read`.

**Missions event** (`isEvent`) : déclarées par le jeu (interne) ou par une corporation autorisée (`mission:event:manage` via Social : CEO ou `manage_corporation`, `POST /:missionId/event`) — cap **1000 assignés** (sinon 100), retirer le flag exige `maxAssignees` ≤ 100 (`400 EVENT_CAPACITY_REQUIRES_FLAG`).

**Multijoueur** : `maxAssignees` = capacité (`1` unique, `N` places limitées, 1000 en event), combinable avec le partage à un groupe (places comptées **au sein du groupe claimant**). Objectifs partagés ; à la complétion, tous les participants actifs sont complétés d'un coup et reçoivent un **part égal de chaque composant de récompense**, figé dans **`rewardShares`** (rejeu de règlement déterministe). Composant **instance unique** ou **item séquestré** refusé en multijoueur (`400 ITEM_REWARD_NOT_SPLITABLE`).

**Missions joueurs** (`POST /api/missions`) : `rewards[]` **séquestrés à la création** — le composant crédits est débité du portefeuille du créateur (`economie:wallet:debit`, `externalId = mission-escrow:<missionId>`), **chaque** composant item est réservé en hold (`escrowItemHoldIds[]`, un par item). Remboursé si annulation/expiration (`mission-refund:<missionId>`). `visibility: corporation` vérifiée via social.

**Financement par un trésor d'organisation** (`escrowSource`) : une mission émise par une **corporation** (`corporationId`) ou une **entité politique** (`politicalEntityId`, commune → fédération ; `issuerType: politics`) peut être prélevée sur **le compte bancaire de l'organisation** plutôt que sur le portefeuille du créateur : `escrowSource: 'issuer'` (défaut `'creator'` = comportement classuel, le joueur paie de sa poche). Garde-fous : le créateur doit être **membre de l'émetteur** et détenir l'action **`mission:treasury:commit`** — accordée par Social au CEO (ou à un grade portant `manage_corporation`) pour une corporation, au chef (ou à un office portant `manage_treasury`) pour une entité politique (`403 TREASURY_FORBIDDEN`, `400 ESCROW_SOURCE_INVALID` si émetteur absent). La décision passe par `POST /api/internal/authorize` (`social:authorize`) : mission n'interprète plus les permissions lui-même. Le remboursement (annulation/expiration) revient sur le même compte (`escrowPayerType` : `player|corporation|politics`, clés `mission-refund:<id>` identiques).

**Séquestre d'une mission interne sur un trésor politique** (`escrow`, `POST /api/internal/missions`) : un service de confiance (ex. `svc-universesimulator` pour la fédération ARES) finance le composant **crédits** depuis le trésor d'une entité politique au lieu du faucet. Exige le rôle **`mission:escrow:politics`** en plus de `mission:write` (`403 FORBIDDEN` sinon) ; pas de contrôle `mission:treasury:commit` (l'appelant est un service). Même cycle de vie que les missions joueurs : débit `mission-escrow:<missionId>` à la création (`escrowStatus: held`, `escrowPayerType: politics`), remboursement `mission-refund:<missionId>` sur le trésor à l'annulation/expiration, `claimed` après règlement. La mission est insérée avant le débit (un `scriptId` dupliqué est refusé sans mouvement d'argent — conseillé pour rendre la création rejouable) puis **supprimée** si le débit échoue. Erreurs : `409 INSUFFICIENT_FUNDS` (trésor insuffisant, rien n'est persisté), `400 ESCROW_ITEM_REWARD_UNSUPPORTED` (récompense item avec `escrow`), `400 ESCROW_CREDITS_REQUIRED` (`escrow` sans composant crédits).

```jsonc
POST /api/internal/missions
{
  "title": "Transport de minerai pour ARES",
  "category": "delivery",               // deliver_items n'est pas autorisé en catégorie transport
  "issuerType": "politics",
  "issuerId": "33333333-3333-3333-3333-333333333333",
  "scriptId": "unisim:transport:42",
  "rewards": [{ "type": "credits", "amount": 4000 }],
  "escrow": { "payerType": "politics", "payerId": "33333333-3333-3333-3333-333333333333" },
  "objectives": [
    { "type": "deliver_items", "title": "Livrer 50 minerais", "targetQuantity": 50,
      "params": { "itemId": "iron_ore", "to": { "holderType": "warehouse", "holderId": "44444444-4444-4444-4444-444444444444" } } }
  ]
}
```

**Créer une mission au nom d'une corporation ou d'une entité politique** : pas d'endpoint dédié — c'est `POST /api/missions` (JWT joueur) qui porte l'émetteur ; `issuerType` est résolu automatiquement (`politics` > `corporation` > `player`, `missions.service.ts`). Le créateur est tracé dans `createdBy` et retrouve ses créations via `GET /api/me/missions-created`.

```jsonc
// Corporation (créateur = membre ; action mission:treasury:commit : CEO ou manage_corporation)
POST /api/missions
{
  "title": "Livraison de minerai",
  "corporationId": "11111111-1111-1111-1111-111111111111",
  "visibility": "corporation",          // limitée aux membres de la corporation (optionnel : "public")
  "escrowSource": "issuer",             // débité du trésor corpo ; "creator" (défaut) = mon portefeuille
  "rewards": [{ "type": "credits", "amount": 5000 }],
  "objectives": [
    { "type": "deliver_material", "title": "Livrer 10 minerais", "targetQuantity": 10,
      "params": { "itemId": "iron_ore" } }
  ],
  "zones": [{ "kind": "system", "system": "sol" }]
}

// Entité politique (créateur = membre ; action mission:treasury:commit : chef ou manage_treasury)
POST /api/missions
{
  "title": "Patrouille de la commune",
  "politicalEntityId": "22222222-2222-2222-2222-222222222222",
  "escrowSource": "issuer",             // débité du trésor politique
  "rewards": [{ "type": "credits", "amount": 2500 }],
  "objectives": [
    { "type": "visit", "title": "Visiter la place du village", "targetQuantity": 1 }
  ]
}
```

Erreurs typées : `403 NOT_CORPORATION_MEMBER` (visibilité corporation hors corp), `400 ESCROW_SOURCE_INVALID` (`issuer` sans `corporationId`/`politicalEntityId`), `403 TREASURY_FORBIDDEN` (pas la permission de dépense), `409 INSUFFICIENT_FUNDS` (trésor insuffisant). À noter : les composants **items** sont toujours séquestrés dans l'inventaire **du créateur** même en `escrowSource: 'issuer'` — le trésor ne finance que le composant crédits ; et sans `escrowSource: 'issuer'`, c'est le créateur qui paie, même si l'émetteur est l'organisation.

**Règlement** : à la complétion, **revérification** des objectifs `service` (sauf `force`) puis paiement composant par composant : crédits idempotents via `externalId = mission:<missionId>:<playerId>` ; items = **claim atomique** dans `settledComponents` (`item:<itemId>`) *avant* le transfert (hold séquestré consommé, sinon faucet `system`) — un échec partiel se rejoue sans jamais double-payer. Rejouer via `POST /api/internal/missions/:missionId/settle`.

## Gameplay (cycles de vie avancés)

**Complétion automatique** (`autoComplete`, défaut `true`) : dès que **tous les objectifs requis** sont faits (push `reportProgress` ou `verify`), la mission se complète et se règle **d'elle-même** — la réponse porte `completed: true` et l'assignation passe `completed`. Avec `autoComplete: false`, le jeu ou le joueur appelle `POST /:missionId/complete`.

**Objectifs bonus** (`isBonus: true`) : hors du gate de complétion, de l'ordre scénarisé **et** du ratio partiel. Un objectif bonus peut porter un **`bonusReward`** (1 composant) accordé **en plus** de la récompense de base, **uniquement sur une complétion totale** (jamais sur un partiel) ; les crédits bonus s'additionnent au composant crédits unique, les items bonus sont ajoutés à la liste. Une mission doit garder **au moins un objectif non-bonus** (`400 MISSING_REQUIRED_OBJECTIVE`) ; un `bonusReward` sur un objectif non-bonus est refusé (`400 BONUS_REWARD_WITHOUT_BONUS`).

**Remise partielle** (`allowPartial`) : `POST /:missionId/complete {partial: true}` (joueur) ou `partial: true` (interne) rend la mission **proportionnellement** à l'avancement pondéré des objectifs requis (`weight`, en points de base : `ratioBp = Σ weight×progress/target / Σ weight × 10000`) :
- l'assignation passe en statut terminal **`partially_completed`** (jamais ré-acceptable : le `rewardExternalId` est déjà consommé) avec `completionRatio` figé ;
- seuls les **crédits** sont payés, **au prorata** (`floor(part × ratioBp / 10000)`) ; **aucun item** n'est versé ;
- la mission est **close** (statut `completed`) et le **solde non utilisé du séquestre est remboursé** au payeur (`mission-refund:<id>`) ;
- refus : mission sans `allowPartial` (`409 PARTIAL_NOT_ALLOWED`), aucun progrès (`409 PARTIAL_NO_PROGRESS`), un autre assigné encore actif (`409 PARTIAL_MULTIPLAYER` — le partiel est un cas solo : le part du joueur est calculée sur la totalité de la mission).

**Expiration & deadlines** : `expiresAt` (absolu) ou `expiresInHours` (relatif) bornent la mission ; sans les deux, le TTL par défaut s'applique (`MISSION_DEFAULT_TTL_HOURS` interne, `MISSION_PLAYER_DEFAULT_TTL_HOURS` joueur). `assignmentDeadlineHours` pose une **deadline par assignation** (`deadlineAt`, calculée à l'acceptation). L'expiration est pilotée **de l'extérieur** : une commande (CronJob Kubernetes) appelle `POST /api/internal/missions/expire`, qui ferme les missions (`expired` + remboursement) et les assignations dépassées (`expired`), en **notifiant** les joueurs une fois (`deadlineNotifiedAt`) — opération **idempotente**, rejouable à la main. Un balayage in-process optionnel (`scheduler.ts`) existe pour le dev monoposte, **désactivé par défaut** (`MISSION_EXPIRE_INTERVAL_SECONDS=0`). Agir sur une assignation dépassée → `409 DEADLINE_PASSED`. Anti-accumulation : un joueur ne dépasse pas `MISSION_MAX_ACTIVE_ASSIGNMENTS` assignations actives (`409 ASSIGNMENT_LIMIT_REACHED`).

**Chaînage de scénarios** (`requiresMissionIds[]`) : une mission ne peut être acceptée que si le joueur a **déjà complété** chaque mission listée (`409 CHAIN_INCOMPLETE`, joueurs uniquement — les assignations PNJ l'ignorent). À la création : ids inconnus (`400 CHAIN_UNKNOWN`), auto-référence (`400 CHAIN_SELF`) et **cycles** (`400 CHAIN_CYCLE`) sont refusés.

**Annulation par le créateur** : `DELETE /api/missions/:missionId` (créateur ou membre de la corporation émettrice) — la mission passe `cancelled`, les assignations actives sont abandonnées, les séquestres (crédits + items) remboursés. Échec si déjà `completed` (`409 MISSION_COMPLETED`).

**Assignations en échec** : une mesure `service` qui bute sur une erreur **permanente** du service cible (Inventory `404`) met l'assignation en **`failed`** (terminal) et notifie le joueur — retenter ne sert à rien.

**Récompenses de standing** (`{type:'standing', domain, amount}`) : le **standing de personnage** (score par domaine, catégorie de mission) est **détenu par Social** (`player_profiles.standing`, colonne `jsonb`) — distinct de `reputation` (modération sociale). À la complétion **totale** (jamais sur un partiel), le delta `amount` est appliqué via `PATCH /api/internal/players/:id/standing` (rol `social:standing:write`), **sans partage** entre les assignés (chaque participant reçoit le montant complet, figé dans `rewardShares`) ; ignoré pour les assignations PNJ. Idempotent : chaque composant standing réclame sa clé `standing:<domain>` dans `settledComponents` avant l'appel Social ; échec Social → `502 SOCIAL_STANDING_FAILED` (assignation rejouable). Le prereq correspondant `min_standing {domain, min}` vérifie le score à l'acceptation (domaine absent = 0).

**Notifications** (`type` + `data`, jamais bloquantes, Social en best-effort) : `mission.expired`, `mission.deadline`, `mission.completed`, `mission.partially_completed`, `mission.failed`, `mission.cancelled`.

## Pagination (rupture de contrat)

Toutes les listes acceptent désormais `?limit=&offset=` et répondent par l'enveloppe
`{ items, total, limit, offset }` au lieu d'un tableau brut :

- `limit` : entier **1..100**, défaut `20`
- `offset` : entier **≥ 0**, défaut `0` (décalage en lignes, pas en pages)
- `items` : la page courante ; `total` : nombre total d'éléments correspondants,
  calculé par une requête `count()` distincte (donc juste même sur une page vide)
- l'ordre est déterministe (tri + `id` en critère d'arbitrage) : une page suivante
  ne saute ni ne répète d'élément

Exception particulière : la clé `missions` est conservée, les compteurs sont ajoutés
à côté :

- `GET /api/missions`, `GET /api/me/missions`, `GET /api/me/missions-created`,
  `GET /api/internal/missions` et `GET /api/internal/players/:id/missions` →
  `{ missions, total, limit, offset }`
- exemptes : le catalogue de kinds (`/api/missions/kinds`) et les détails de mission

## Endpoints

Spécification complète (schémas, codes d'erreur) : [`openapi.yaml`](openapi.yaml).

Erreurs : `{ "error": "CODE", "message": "...", "status": 4xx }`.

Langue : envoyez **`Accept-Language: fr`** (ou `en`, **défaut `en`**) — les messages d'erreur (codes et texte dynamique) sont rendus dans la langue demandée ; les messages de validation zod restent en anglais. Les identifiants techniques (codes, kinds, champs, `{params}`) ne sont jamais traduits.

### Public
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/health` | Liveness (`?deep=1` vérifie aussi le schéma en base → `503` si absent) |
| GET | `/openapi.yaml` | Document OpenAPI du service (aussi exposé sur `/api/openapi.yaml`) |

### Joueur (`Authorization: Bearer <JWT Keycloak>`)
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/missions/kinds` | Catalogue des kinds (catégories, objectifs, prérequis, `name` localisé, forme des récompenses) pour les builders |
| POST | `/api/missions/validate` `{mode?, mission}` | Dry-run d'une spec (aucune persistance, aucun séquestre) |
| GET | `/api/missions?status=&kind=&category=&issuerType=&issuerId=&visibility=&groupClaimable=&isEvent=&limit=&offset=` | Missions **avec une place libre** (statuts `available` + `active` par défaut ; groupes partagés visibles membres uniquement ; **zones** : sans position connue, seules les missions globales sont listées) |
| POST | `/api/missions` `{title, rewards[], prerequisites?, zones?, visibility?, corporationId?, politicalEntityId?, escrowSource?, maxAssignees?, groupClaimable?, objectives[{type(kind), params?…}]}` | Créer une mission (récompenses séquestrées ; `escrowSource: 'issuer'` = prélevé sur le **trésor corpo/politique**) |
| GET | `/api/missions/:missionId` | Détail + objectifs + mon assignation (404 si partagée à un groupe dont je ne suis pas membre) |
| DELETE | `/api/missions/:missionId` | **Annuler ma mission** (créateur ou membre corp ; séquestres remboursés, `409 MISSION_COMPLETED` si déjà complétée) |
| POST | `/api/missions/:missionId/accept` | Accepter : corp + **zone** (`403 OUT_OF_ZONE`) + **prérequis** (`403 PREREQ_FAILED`) + groupe (`NOT_GROUP_MEMBER`, `NO_GROUP`, `GROUP_ALREADY_CLAIMED`) |
| POST | `/api/missions/:missionId/verify` | Re-mesurer les objectifs `service` (effectue les livraisons `deliver_items`) |
| POST | `/api/missions/:missionId/objectives/:objectiveId/confirm` | Confirmer un objectif `manual` (**créateur** de la mission uniquement) |
| POST | `/api/missions/:missionId/share` `{groupId}` | Partager la mission à un groupe (créateur ou membre corp ; `409 MISSION_HAS_ASSIGNEES`) |
| DELETE | `/api/missions/:missionId/share` | Retirer le partage (mêmes règles) |
| POST | `/api/missions/:missionId/event` `{enabled, maxAssignees?}` | Déclarer/retirer le flag **event** (mission corporation, action `mission:event:manage` : CEO ou `manage_corporation`) |
| POST | `/api/missions/:missionId/abandon` | Abandonner l'assignation |
| POST | `/api/missions/:missionId/objectives/:objectiveId/progress` `{quantity}` | Reporter la progression d'un objectif **`game`** uniquement (`409 OBJECTIVE_NOT_PUSHABLE` sinon) |
| POST | `/api/missions/:missionId/complete` `{partial?}` | Compléter : **revérifie** les objectifs `service`, puis répartit chaque composant de récompense (`partial: true` = remise proportionnelle, missions `allowPartial`) |
| GET | `/api/me/missions?status=&limit=&offset=` | Mes missions (assignations + mission) |
| GET | `/api/me/missions-created?status=&limit=&offset=` | **Mes missions créées** (`createdBy` = moi), tous statuts, **sans filtre zone/groupe/place libre** — vue de gestion du créateur (le browse `/api/missions` reste filtré par zone) |

### Interne — serveur de jeu (token Keycloak de service + rôle de capacité)
| Méthode | Route | Rôle requis | Description |
|---|---|---|---|
| POST | `/api/internal/missions` | `mission:write` (+ `mission:escrow:politics` si `escrow`) | Créer une mission (`rewards[]`, `prerequisites[]`, `zones[]`, kinds, `groupClaimable`, `isEvent`, `escrow?`) |
| GET | `/api/internal/missions?status=&kind=&category=&issuerType=&issuerId=&groupClaimable=&isEvent=&limit=&offset=` | `mission:read` | Catalogue complet (tous statuts, sans filtre zone) |
| GET | `/api/internal/missions/:missionId` | `mission:read` | **Détail complet** pour le panneau admin : mission, objectifs ordonnés, **toutes** les assignations, noms joueurs + groupe (Social best-effort, cache 60 s), chaîne requise, synthèse (compte par statut + `progressBp`) |
| PATCH | `/api/internal/missions/:missionId` | `mission:write` | Modifier **titre, description, expiration, `isEvent`, `zones`** seulement (spec immuable ; zones sans impact séquestre) |
| POST | `/api/internal/missions/:missionId/share` `{groupId}` | `mission:write` | Partager la mission à un groupe (canal de confiance) |
| DELETE | `/api/internal/missions/:missionId/share` | `mission:write` | Retirer le partage |
| POST | `/api/internal/missions/:missionId/event` `{enabled, maxAssignees?}` | `mission:write` | Déclarer/retirer le flag **event** (pas de vérif corporation) |
| POST | `/api/internal/missions/:missionId/cancel` | `mission:write` | Annuler la mission, ses assignations actives et **rembourser le séquestre** |
| POST | `/api/internal/missions/expire` | `mission:write` | Expire missions **et assignations** dépassées (balayage intégré, idempotent) → `{expired, expiredAssignments}` |
| POST | `/api/internal/missions/:missionId/assign` `{playerId, holderType?}` | `mission:write` | Assigner une mission à un détenteur (`player` ou `npc`) |
| POST | `/api/internal/missions/:missionId/objectives/:objectiveId/progress` `{playerId, quantity}` | `mission:progress` | Progression **vérifiée** côté serveur de jeu (kinds `game`) |
| POST | `/api/internal/missions/:missionId/objectives/:objectiveId/confirm` `{}` | `mission:progress` | Confirmer un objectif `manual` (mission créée par le jeu/corp) |
| POST | `/api/internal/missions/:missionId/verify` `{playerId}` | `mission:progress` | Re-mesurer les objectifs `service` d'un détenteur |
| POST | `/api/internal/missions/:missionId/complete` `{playerId, force?, settle?, partial?}` | `mission:complete` | Compléter tous les participants (revérifie ; `force` pour outrepasser, `partial` pour une remise proportionnelle) |
| POST | `/api/internal/missions/:missionId/settle` `{playerId}` | `mission:complete` | Rejouer le règlement des composants non réglés (idempotent) |
| GET | `/api/internal/players/:playerId/missions` | `mission:read` | Assignations d'un joueur (+ mission) |

**Auth de service** : le garde est monté sur le préfixe (`app.use('/api/internal', serviceAuth, …)`), donc toute route ajoutée ici reste protégée. Le token doit venir d'un des clients `INTERNAL_SERVICE_CLIENTS` (`azp`), viser `OIDC_SERVICE_AUDIENCE`, puis détenir le rôle de capacité de la route — sinon `403 SERVICE_FORBIDDEN`/`FORBIDDEN`. Un token joueur ne peut jamais avoir un `azp` de service : c'est la garantie de non-contournement.

### Configuration Keycloak requise
- Client de service `svc-game` (serveur de jeu) autorisé dans `INTERNAL_SERVICE_CLIENTS`, avec les rôles `mission:read`, `mission:write`, `mission:progress`, `mission:complete`.
- Client de service `svc-universesimulator` (simulateur d'univers, missions de transport/minage au nom d'une fédération) autorisé dans `INTERNAL_SERVICE_CLIENTS`, avec `mission:read`, `mission:write` **et `mission:escrow:politics`**.

| Rôle de service | Accorde |
|---|---|
| `mission:read` | Lecture du catalogue et des assignations (`GET /api/internal/...`) |
| `mission:write` | Création/modification/annulation/partage/assignation de missions |
| `mission:progress` | Progression, confirmation et re-mesure des objectifs |
| `mission:complete` | Complétion et règlement des récompenses |
| `mission:escrow:politics` | Financer la récompense crédits d'une mission interne sur un **trésor politique** (`escrow` à la création, en plus de `mission:write`) |
- Compte de service **`svc-mission`** (celui de ce service) — rôles à accorder sur les clients cibles :
  - **economie** (`INTERNAL_SERVICE_CLIENTS`) : `economie:wallet:credit`, `economie:wallet:debit` (séquestre) **et `economie:wallet:read`** (prérequis/objet `has_credits`) ;
  - **inventory** : **`inventory:read`** (mesure `owns_items`/`deliver_items` + prérequis) — les rôles de hold/transfer/credit utilisés par le règlement doivent aussi être accordés ;
  - **social** : `social:corporation:read` (missions corporation), `social:group:read` (partage/claim de groupes), `social:profile:read` (prérequis `min_reputation`) **et `social:politics:read`** (appartenance politique pour `escrowSource: issuer`) ;
  - **economie** : **`economie:politics:manage`** (débit/crédit du trésor politique — le trésor corporation n'a besoin d'aucun rôle supplémentaire, `economie:wallet:*` suffit).
- Création des clients/secrets/rôles gérée côté Keycloak (realm `dyingstar`, clients `svc-*`), hors realm JSON.

## Structure

```
src/
  index.ts          bootstrap Express, migrations, listen
  config/env.ts     variables d'environnement
  db/schema/        tables drizzle (missions, mission_objectives, mission_assignments)
  db/connection.ts  pool pg + drizzle ; db/migrate.ts applique ./drizzle (migration unique `0000_init`) et refuse de démarrer si le schéma manque
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
- [x] Mission avec objectif vérifiable (livraison de matériaux, transport A→B, visite)
- [x] Cycle de vie joueur : accepter, progresser, compléter, abandonner
- [x] Missions scénarisées avec objectifs ordonnés (verrouillage séquentiel)
- [x] Récompense économique ou item
- [x] Récompense item réglée via le service inventory (séquestre à la création pour les missions joueur)
- [x] Missions multijoueurs : objectifs partagés et récompense répartie entre participants
- [x] Capacité (`maxAssignees`) : mission unique (1), places limitées (N) ou event (jusqu'à 1000)
- [x] Les missions sans place libre disparaissent de la liste des missions dispo
- [x] Missions réservées à 1 groupe (premier groupe claimant) et partage de toute mission à un groupe
- [x] Missions event déclarées par le jeu ou une corporation (action `mission:event:manage`)
- [x] **Complétion automatique** (`autoComplete`) — la mission se règle seule quand les objectifs requis sont faits
- [x] **Objectifs bonus** (`isBonus` + `bonusReward`) hors gate/ratio, récompensés en plus sur complétion totale
- [x] **Remise partielle proportionnelle** (`allowPartial` + `partial`) : crédits au prorata pondéré, items jamais payés, séquestre remboursé, assignation `partially_completed`
- [x] **Expiration** : TTL par défaut joueur/interne, `expiresInHours`, **deadline par assignation** (`assignmentDeadlineHours`), balayage intégré + notifications
- [x] **Chaînage de scénarios** (`requiresMissionIds` : gate d'acceptation, anti-cycle à la création)
- [x] **Annulation par le créateur** (`DELETE /api/missions/:missionId`)
- [x] **Anti-accumulation** : plafond d'assignations actives par joueur
- [x] **Assignations en échec** (`failed` sur erreur permanente d'Inventory)
- [x] **Détail interne complet** pour le panneau admin (`GET /api/internal/missions/:missionId`)
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
- [x] Missions financées par un **trésor d'organisation** (`escrowSource: 'issuer'` : action `mission:treasury:commit` décidée par Social)
- [x] Missions **zonées** : dispo par système ou **POI** (`inventory` : scène hiérarchique / aire / point) — filtrées au listing et à l'acceptation (`403 OUT_OF_ZONE`) selon la présence social
- [x] Spec immuable après création (précédent FTB : le séquestre est pris dessus)
- [x] **Break propre** : ancienne forme `reward` retirée, serveur de jeu à adapter (corps `rewards[]`, `params`, verify/confirm)
- [ ] Composition booléenne des prérequis (all/any/none), caution du contractor, missions répétables (phase 2) — les **chaînes** sont livrées via `requiresMissionIds`
- [ ] Kind `traded_items` (vérifié via market) — phase 2

### Missions dynamiques
- [ ] Génération à partir des besoins en ressources des corporations IA / villes (via l'API economie)
- [x] Missions de livraison avec vérification de la quantité livrée avant paiement (`deliver_items`)
- [x] Expiration périodique (TTL + deadlines, balayage intégré)
- [ ] Régénération/rotation périodique des missions dynamiques

### Missions joueurs
- [x] Création de missions par les joueurs (récompense économique et/ou item séquestrée)
- [x] Visibilité publique ou réservée à une corporation (via l'API interne sociale)
- [~] Missions privées (invitation de joueurs précis) — le **partage à un groupe** (groupe temporaire Social) est fait ; l'invitation de joueurs nommés reste à venir

### Récompenses
- [x] Paiement économique via l'API interne economie (idempotent)
- [x] Séquestre des récompenses des missions joueurs (débit/remboursement ; hold item/release)
- [x] Récompenses items via le service inventory (hold/consume ou transfert depuis `system`)
- [x] Récompenses de **standing de personnage** (`{type:'standing', domain, amount}` — Social détient `player_profiles.standing`, delta non splitté, jamais sur un partiel, prereq `min_standing`) ; la réputation de modération reste gérée à part (service Social)
