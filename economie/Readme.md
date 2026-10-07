# Service economie

API économique de DyingStar : comptes joueurs, corporations et entités politiques, portefeuilles multi-devises, transactions, trésoreries (corporation & politique), fiscalité « en dette » et création monétaire (à terme : marché, conversion, régulation et analytique).
Stack : Node 22, TypeScript, Express 4, drizzle-orm + PostgreSQL, JWT Keycloak validé via JWKS (`jose`). Structure et conventions calquées sur le service [`social`](../social/).

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

Autres scripts : `pnpm build` / `pnpm start` (prod), `pnpm type-check`, `pnpm db:generate` (après modification de `src/db/schema/`), `pnpm db:studio`, `pnpm db:reset -- --yes` (vide `public` + le schéma `drizzle` : on repart de zéro, la migration unique `0000_init` est rejouée au démarrage).

Sans Keycloak, mettre `AUTH_DEV_BYPASS=true` (ignoré en production) et passer `X-Player-Id: <uuid>` (+ `X-Player-Name`) à la place du bearer :

```bash
curl localhost:3000/api/me/wallet -H "X-Player-Id: 11111111-1111-4111-8111-111111111111" -H "X-Player-Name: alice"
```

Pour l'API interne sans Keycloak, mettre `INTERNAL_DEV_BYPASS=true` (ignoré en production, où le service refuse de démarrer si le flag est actif) et passer `X-Internal-Key: <INTERNAL_API_KEY>`. En production, `/api/internal/*` n'accepte **qu'un** token de service Keycloak :

```bash
TOKEN=$(curl -s -X POST "$OIDC_TOKEN_URL" -d grant_type=client_credentials \
  -d client_id=svc-game -d client_secret="$SVC_GAME_CLIENT_SECRET" | jq -r .access_token)
curl localhost:3000/api/internal/players/$ID/wallet -H "Authorization: Bearer $TOKEN"
```

## Variables d'environnement

| Variable | Rôle |
|---|---|
| `PORT`, `NODE_ENV`, `CORS_ORIGIN` | HTTP |
| `DATABASE_URL` | PostgreSQL (obligatoire) |
| `OIDC_ISSUER`, `OIDC_JWKS_URL`, `OIDC_AUDIENCE` | Validation des JWT Keycloak joueurs (`player_id` = claim `sub`) |
| `OIDC_SERVICE_AUDIENCE` | `aud` attendu des tokens de service sur `/api/internal/*` (défaut `economie-api`) |
| `INTERNAL_SERVICE_CLIENTS` | Clients Keycloak de service autorisés (`azp`, CSV) sur `/api/internal/*` |
| `INTERNAL_API_KEY` | Secret hérité de `X-Internal-Key` — **dev uniquement** (voir `INTERNAL_DEV_BYPASS`) |
| `INTERNAL_DEV_BYPASS` | Accepter `X-Internal-Key` sur `/api/internal/*` (ignoré en production) |
| `AUTH_DEV_BYPASS` | Accepter `X-Player-Id` (+ `X-Player-Name`, `X-Player-Roles`) sans JWT (dev uniquement) |
| `ECONOMY_TRANSFER_TAX_BPS` | Taxe automatique sur les transferts joueurs (basis points ; 500 = 5 %), prélevée côté payeur |
| `ECONOMY_TRANSFER_TAX_CEILING` | Plafond absolu de la taxe par transfert (0 = aucun) |
| `ECONOMY_MIN_TRANSFER` / `ECONOMY_MAX_TRANSFER` | Bornes d'un transfert (0 = pas de plafond) |
| `ECONOMY_TAX_VAULT_UUID` | Compte système réservé qui encaisse les taxes automatiques |
| `SOCIAL_API_URL` | URL interne du service social (résolution des pseudos pour l'admin **+ ACL** `economie:treasury:manage` ; vide = désactivé) |
| `SOCIAL_SERVICE_CLIENT_ID` / `SOCIAL_SERVICE_CLIENT_SECRET` | Compte de service Keycloak pour appeler social (`client_credentials`) |
| `SOCIAL_INTERNAL_API_KEY` | Repli dev : clé partagée envoyée en `X-Internal-Key` si aucun secret n'est défini |

Montants en **unités entières** (crédits). Une devise est une simple chaîne (`credits` = monnaie universelle) ; un porteur possède un compte par devise (`unique(holder_type, holder_id, currency)`). Types de porteur : `player`, `npc`, `corporation`, `system` — les PNJ disposent donc de portefeuilles à parité avec les joueurs.

## Pagination (rupture de contrat)

Toutes les listes acceptent désormais `?limit=&offset=` et répondent par l'enveloppe
`{ items, total, limit, offset }` au lieu d'un tableau brut :

- `limit` : entier **1..100**, défaut `20`
- `offset` : entier **≥ 0**, défaut `0` (décalage en lignes, pas en pages)
- `items` : la page courante ; `total` : nombre total d'éléments correspondants,
  calculé par une requête `count()` distincte (donc juste même sur une page vide)
- l'ordre est déterministe (tri + `id` en critère d'arbitrage) : une page suivante
  ne saute ni ne répète d'élément

Exceptions & particularités :

- `GET /api/corporations/:id/salaries` : `roleDefaults` reste complet, `memberOverrides`
  est paginé — ajouts `memberOverridesTotal`, `limit`, `offset`
- `GET /api/me/wallet/transactions`, `GET /api/me/taxes`, listes de membres, dettes fiscales
  et journaux (internes compris) → enveloppe `{ items, total, limit, offset }`
- `GET /api/admin/players` → enveloppe (résultats issus de Social)
- exemptes : `GET /api/corporations/:id/report` et `GET /api/admin/stats` (agrégats),
  comptes wallet (`{ accounts: [...] }`, un compte par devise)

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
| GET | `/api/me/wallet` | Mes comptes (un par devise, `credits` créé au premier appel) |
| GET | `/api/me/wallet/transactions?limit=&offset=` | Mon historique (toutes devises, plus récent d'abord) |
| POST | `/api/transfers` `{toPlayerId, amount, memo?}` | Transfert direct (taxe automatique `ECONOMY_TRANSFER_TAX_BPS` à la charge de l'émetteur) |

### Corporations (`Authorization: Bearer <JWT Keycloak>`) — adhésion renseignée par le serveur via l'API interne
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/corporations/:corporationId/wallet` | Trésorerie (membre) |
| GET | `/api/corporations/:corporationId/wallet/transactions?limit=&offset=` | Journal de trésorerie (membre) |
| GET | `/api/corporations/:corporationId/members` | Membres et rôles (membre) |
| GET | `/api/corporations/:corporationId/report?from=&to=` | Bilan financier par devise et par type (`economie:treasury:manage`) |
| POST | `/api/corporations/:corporationId/donations` `{amount, memo?}` | Don d'un membre (respecte `allowDonations`, taxe interne `taxRateBps`) |
| GET | `/api/corporations/:corporationId/salaries` | Salaires par rôle + overrides par membre (`economie:treasury:manage`) |
| PUT | `/api/corporations/:corporationId/salaries/roles/:role` `{amount, currency?, enabled?}` | Définir le salaire par défaut d'un rôle (`economie:treasury:manage`) |
| PUT | `/api/corporations/:corporationId/salaries/members/:playerId` `{amount, currency?, enabled?}` | Définir un override de salaire pour un membre (`economie:treasury:manage`) |
| DELETE | `/api/corporations/:corporationId/salaries/members/:playerId` | Retirer un override (retour au salaire du rôle) |
| POST | `/api/corporations/:corporationId/payroll?currency=` | Verser les salaires à tous les membres éligibles (`economie:treasury:manage`, atomique) |
| POST | `/api/corporations/:corporationId/members/:playerId/prime` `{amount, currency?, memo?}` | Verser une prime ponctuelle à un membre (`economie:treasury:manage`) |

Rôles de trésorerie : `leader` > `treasurer` > `member`. Un membre peut être un joueur ou un PNJ (`holderType`) ; les salaires et primes sont versés sur le portefeuille correspondant. Ces rôles restent la source locale des **paliers de salaire** et du `holderType`.

**Qui opère la trésorerie** est décidé par [`social`](../social/Readme.md) : les 8 routes ci-dessus appellent `requireTreasuryPermission` → `POST /api/internal/authorize` avec l'action **`economie:treasury:manage`** (`403 NOT_CORPORATION_MEMBER` si non-membre, `403 FORBIDDEN` sinon). Cette action n'a **aucun `satisfiedBy`** : seul le CEO (ou un grade à qui elle est accordée explicitement) passe — voir `KEYCLOAK_SOCIAL_AUTHORIZE.md` §4 pour le seed des grades. Social non configuré → `503 SOCIAL_NOT_CONFIGURED` (sauf `AUTH_DEV_BYPASS=true`, où le joueur authentifié est cru).

### Politique — trésorerie, taxes & émission (`Authorization: Bearer`)
Chaque entité politique (commune, agglomération, département, région, pays, fédération — possédée par le service [`social`](../social/Readme.md)) dispose d'une **trésorerie** (compte `political`). La fiscalité fonctionne **comme un loyer** : une **assiette** calcule et **inscrit une dette**, puis le **redevable** déclenche le paiement. La **création monétaire** est réservée aux pays/fédérations via le serveur de jeu.

| Méthode | Route | Description |
|---|---|---|
| GET | `/api/me/taxes` | Mes dettes fiscales (joueur) |
| POST | `/api/me/taxes/pay` `{currency?, entityId?}` | Régler mes dettes échues payables (partiel toléré) |
| GET | `/api/corporations/:corporationId/taxes` | Dettes fiscales de la corporation (membre) |
| POST | `/api/corporations/:corporationId/taxes/pay` `{currency?, entityId?}` | Régler les dettes de la corporation (`economie:treasury:manage`) |

**Assiette** (`POST /api/internal/politics/:entityId/taxes/assess`) : taxe corporative = `corporateTaxBps` × solde de trésorerie des corporations **rattachées** (`politicalEntityId`) ; impôt citoyen = `incomeTaxBps` × revenus (`salary`, `prime`, `mission_reward`) reçus par les membres depuis la dernière assiette. Le paiement débite le portefeuille du redevable et crédite la trésorerie politique (`type: tax`). Solde insuffisant → les dettes payables sont réglées, les autres restent dues (`409 INSUFFICIENT_FUNDS` si aucune ne l'est).

**Émission** (`POST /api/internal/politics/:entityId/mint`, rôle `economie:money:issue`) : crée de la monnaie et la crédite à la trésorerie de l'entité (`type: issuance`), sous réserve de `allowMinting` et du plafond `mintCeiling`. L'émission augmente la **masse monétaire** (`/api/admin/stats`).

### Admin — tableau de bord (`Authorization: Bearer <JWT>, rôle moderator+`)
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/admin/stats?days=&top=` | Analytique : masse monétaire, volume/taxes, classements des plus riches (pseudos résolus via social), série journalière |
| GET | `/api/admin/players?search=&limit=&offset=` | Rechercher des joueurs **par pseudo** (proxy vers l'API interne de social) et renvoyer leur portefeuille |

### Interne — serveur de jeu (token Keycloak de service + rôle de capacité)
| Méthode | Route | Rôle requis | Description |
|---|---|---|---|
| PUT | `/api/internal/players/:playerId/wallet` | `economie:wallet:ensure` | Créer le compte `credits` au login (idempotent) |
| GET | `/api/internal/players/:playerId/wallet` | `economie:wallet:read` | Comptes du joueur |
| GET | `/api/internal/players/:playerId/wallet/transactions?limit=&offset=` | `economie:wallet:read` | Historique du joueur |
| POST | `/api/internal/players/:playerId/wallet/credit` `{amount, currency?, reference?, externalId?, type?}` | `economie:wallet:credit` | Créditer (prime de mission, salaire…) |
| POST | `/api/internal/players/:playerId/wallet/debit` | `economie:wallet:debit` | Débiter (refusé si solde insuffisant) |
| PUT | `/api/internal/npcs/:npcId/wallet` | `economie:wallet:ensure` | Créer le compte `credits` d'un PNJ (idempotent) |
| GET | `/api/internal/npcs/:npcId/wallet` | `economie:wallet:read` | Comptes du PNJ |
| GET | `/api/internal/npcs/:npcId/wallet/transactions?limit=&offset=` | `economie:wallet:read` | Historique du PNJ |
| POST | `/api/internal/npcs/:npcId/wallet/credit` `{amount, currency?, reference?, externalId?, type?}` | `economie:wallet:credit` | Créditer un PNJ (salaire, prime, vente…) |
| POST | `/api/internal/npcs/:npcId/wallet/debit` | `economie:wallet:debit` | Débiter un PNJ (refusé si solde insuffisant) |
| PUT | `/api/internal/corporations/:corporationId/wallet` | `economie:wallet:ensure` | Créer le compte `credits` de la corporation |
| GET | `/api/internal/corporations/:corporationId/wallet` | `economie:wallet:read` | Comptes de la trésorerie |
| GET | `/api/internal/corporations/:corporationId/wallet/transactions?limit=&offset=` | `economie:wallet:read` | Journal de trésorerie |
| POST | `/api/internal/corporations/:corporationId/wallet/credit` `/debit` | `economie:wallet:credit` / `:debit` | Mouvements de trésorerie |
| PUT | `/api/internal/corporations/:corporationId/members/:playerId` `{role, holderType?}` | `economie:corporation:manage` | Ajouter/mettre à jour un membre (leader/trésorier/member ; `holderType` = `player`\|`npc`) |
| DELETE | `/api/internal/corporations/:corporationId/members/:playerId` | `economie:corporation:manage` | Retirer un membre |
| GET | `/api/internal/corporations/:corporationId/members` | `economie:corporation:read` | Membres de la trésorerie |
| GET | `/api/internal/corporations/:corporationId/settings` | `economie:corporation:read` | Taxe interne et politique de dons |
| PUT | `/api/internal/corporations/:corporationId/settings` `{taxRateBps?, allowDonations?}` | `economie:corporation:manage` | Régler la taxe interne / les dons |
| PUT | `/api/internal/corporations/:corporationId/affiliation` `{politicalEntityId: uuid\|null}` | `economie:corporation:manage` | Définir le **siège fiscal** (entité politique) d'une corporation |
| PUT/GET | `/api/internal/politics/:entityId/wallet` | `economie:wallet:ensure` / `economie:politics:read` | Créer / lire la trésorerie politique |
| GET | `/api/internal/politics/:entityId/wallet/transactions?limit=&offset=` | `economie:politics:read` | Journal de la trésorerie politique |
| POST | `/api/internal/politics/:entityId/wallet/credit` `/debit` | `economie:politics:manage` | Mouvements de trésorerie politique |
| POST | `/api/internal/politics/:entityId/taxes/assess` `{currency?}` | `economie:politics:manage` | Calculer et inscrire les dettes fiscales |
| GET | `/api/internal/politics/:entityId/settings` | `economie:politics:read` | Taux (`corporateTaxBps`, `incomeTaxBps`) et politique d'émission |
| PUT | `/api/internal/politics/:entityId/settings` `{corporateTaxBps?, incomeTaxBps?, allowMinting?, mintCeiling?}` | `economie:politics:manage` | Régler les taux / l'émission |
| PUT/DELETE | `/api/internal/politics/:entityId/members/:playerId` `{role, holderType?}` | `economie:politics:manage` | Miroir des membres (head/treasurer/member) |
| GET | `/api/internal/politics/:entityId/members` | `economie:politics:read` | Membres de la trésorerie politique |
| POST | `/api/internal/politics/:entityId/mint` `{amount, currency?, reason?}` | `economie:money:issue` | **Créer de la monnaie** (pays/fédération) et créditer la trésorerie |
| GET | `/api/internal/npcs/:npcId/taxes` | `economie:politics:read` | Dettes fiscales du PNJ |
| POST | `/api/internal/npcs/:npcId/taxes/pay` `{currency?, entityId?}` | `economie:politics:manage` | Régler les dettes fiscales du PNJ |

**Auth de service** : le garde est monté sur le préfixe (`app.use('/api/internal', serviceAuth, …)`), donc toute route ajoutée ici reste protégée. Le token doit venir d'un des clients `INTERNAL_SERVICE_CLIENTS` (`azp`), viser `OIDC_SERVICE_AUDIENCE`, puis détenir le rôle de capacité de la route — sinon `403 SERVICE_FORBIDDEN`/`FORBIDDEN`. Un token joueur ne peut jamais avoir un `azp` de service (seul son `client_secret` permet de le frapper), c'est la garantie de non-contournement. Révocation : désactiver le client dans Keycloak (TTL de 300 s côté token). Un crédit/débit de service est tracé dans `transactions.caller` (client Keycloak appelant).

**Idempotence** : les crédits/débits et transferts acceptent `externalId` (clé unique) — le serveur de jeu peut rejouer une écriture sans double comptabilisation (`409 DUPLICATE_EXTERNAL_ID` si déjà enregistrée).

**Comptes système** : les taxes automatiques sont encaissées sur le compte réservé `system` (`ECONOMY_TAX_VAULT_UUID`), créé à la demande. Solde jamais négatif (`CHECK` en base + `409 INSUFFICIENT_FUNDS`).

## Structure

```
src/
  index.ts          bootstrap Express, migrations, listen
  config/env.ts     variables d'environnement
  db/schema/        tables drizzle (accounts, transactions, corporation_*, political_*, salaires)
  db/connection.ts  pool pg + drizzle ; db/migrate.ts applique ./drizzle (migration unique `0000_init`) et refuse de démarrer si le schéma manque
  middleware/       auth (JWT joueur / service-account + rôles de capacité), validate (zod), errorHandler
  routes/           un routeur par ressource, schémas zod dans routes/schemas.ts
  services/         logique métier (une fonction exportée par cas d'usage) ; social.client.ts (pseudos + `authorizeAction`)
```

Déploiement : `docker/Dockerfile` (image standalone, migrations au démarrage), workflows `.github/workflows/build-*-economie.yaml`.

## Feuille de route

Ce service est dédié à la partie économique du jeu ; les features ci-dessous sont traitées pas à pas, en privilégiant celles reliées au service social (corps de métiers, trésorerie de corporation).

### Monnaie & Comptes
- [x] Comptes joueurs et corporations distincts (un compte par devise)
- [x] Comptes PNJ (`npc`) à parité avec les joueurs (portefeuilles, crédits/débits internes)
- [x] Transferts directs entre joueurs avec taxe automatique
- [x] Historique détaillé des transactions (ledger, taxe/frais, `externalId`)
- [x] Coffre système encaissant les taxes automatiques
- [ ] Frais de conversion variables selon le système ou la faction locale (multi-devises + taux)
- [ ] Coffres verrouillés (crédits bloqués, retraits restreints)

### Économie Sociale & Corporations
- [x] Trésorerie de corporation (compte commun) et rôles leader/trésorier/membre
- [x] Dons des membres (politique `allowDonations`, taxe interne `taxRateBps`)
- [x] Bilan financier détaillé (revenus, dépenses, par devise et par type)
- [x] Salaires des membres : montant par défaut par rôle + override par membre, versement manuel leader/trésorier
- [x] Primes ponctuelles versées par la trésorerie à un membre
- [ ] Classement économique des corporations (richesse, stabilité, influence)
- [ ] Système de sponsoring ou mécénat entre corporations

### Trésorerie & Fiscalité politique
- [x] Trésorerie politique (compte `political`) et miroir des membres (head/trésorier/membre)
- [x] Configuration fiscale par entité (`corporateTaxBps`, `incomeTaxBps`) et siège fiscal des corporations
- [x] Assiette manuelle « en dette » (taxe corporative sur la richesse, impôt sur les revenus)
- [x] Paiement volontaire par le redevable (corporation, joueur, PNJ), partiel toléré
- [x] Création monétaire (émission) réservée aux pays/fédérations (`allowMinting`, `mintCeiling`), tracée `issuance`
- [ ] Relances/pénalités de retard, annulation de dette, taxe foncière sur les revenus, injection ciblée (stimulus)

### Régulation & Sécurité (à venir)
- [ ] Détection de fraude / farming abusif
- [ ] Blocage automatique des transactions suspectes
- [ ] Sanctions économiques (pénalités, taxes forcées)
- [ ] Réputation économique personnelle (fiabilité, dette, solvabilité)

### Analytique & Statistiques (à venir)
- [x] Masse monétaire totale, volume des transactions, taxes collectées
- [x] Classement des joueurs / corporations les plus riches
- [x] Tableaux de bord internes (`/api/admin/stats`, rôle `moderator`+)
- [ ] Inflation et tableaux de bord publics
- [ ] Séries temporelles configurables (granularité horaire, export)

### API & Intégrations
- [x] API interne synchronisée avec les serveurs du jeu (transactions, gains, marchés)
- [x] Droits de trésorerie délégués à l'ACL centralisé (`economie:treasury:manage` via `POST /api/internal/authorize`)
- [x] Auth service-à-service par comptes de service Keycloak et rôles de capacité (fin des secrets partagés)
- [~] API publique sécurisée (JWT Keycloak en place ; clés tierces à venir)
- [ ] Conversion de devises et marché (prix, historique, commissions dynamiques)
- [ ] Webhooks sur les variations économiques
- [ ] Intégration au Service Social : affichage du rang économique, badges financiers, trésorerie, salaires automatiques, primes de mission