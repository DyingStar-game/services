# Service social

API sociale de DyingStar : profils joueurs, amis, présence, corporations, politique, réputation et modération.
Stack : Node 22, TypeScript, Express 4, drizzle-orm + PostgreSQL, JWT Keycloak validé via JWKS (`jose`).

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

Autres scripts : `pnpm build` / `pnpm start` (prod), `pnpm type-check`, `pnpm db:generate` (après modification de `src/db/schema/`), `pnpm db:studio`.

Sans Keycloak, mettre `AUTH_DEV_BYPASS=true` (ignoré en production) et passer `X-Player-Id: <uuid>` (+ `X-Player-Name`) à la place du bearer :

```bash
curl localhost:3000/api/me -H "X-Player-Id: 11111111-1111-4111-8111-111111111111" -H "X-Player-Name: alice"
```

Pour l'API interne sans Keycloak, mettre `INTERNAL_DEV_BYPASS=true` (ignoré en production, où le service refuse de démarrer si le flag est actif) et passer `X-Internal-Key: <INTERNAL_API_KEY>`. En production, `/api/internal/*` n'accepte **qu'un** token de service Keycloak (`client_credentials`, `azp` autorisé, `aud` = `social-api`, rôle de capacité requis) : la création des clients de service, de leurs secrets et de leurs rôles de capacité est gérée côté Keycloak (realm `dyingstar`, clients `svc-*`).

## Variables d'environnement

| Variable | Rôle |
|---|---|
| `PORT`, `NODE_ENV`, `CORS_ORIGIN` | HTTP |
| `DATABASE_URL` | PostgreSQL (obligatoire) |
| `OIDC_ISSUER`, `OIDC_JWKS_URL`, `OIDC_AUDIENCE` | Validation des JWT Keycloak joueurs (`player_id` = claim `sub`) |
| `OIDC_SERVICE_AUDIENCE` | `aud` attendu des tokens de service sur `/api/internal/*` (défaut `social-api`) |
| `INTERNAL_SERVICE_CLIENTS` | Clients Keycloak de service autorisés (`azp`, CSV) sur `/api/internal/*` |
| `INTERNAL_API_KEY` | Secret hérité de `X-Internal-Key` — **dev uniquement** (voir `INTERNAL_DEV_BYPASS`) |
| `INTERNAL_DEV_BYPASS` | Accepter `X-Internal-Key` sur `/api/internal/*` (ignoré en production) |
| `AUTH_DEV_BYPASS` | Accepter `X-Player-Id` (+ `X-Player-Name`, `X-Player-Roles`) sans JWT (dev uniquement) |
| `REPUTATION_*` | Pénalités (blocage, signalement, signalement confirmé), seuils de sanctions automatiques (`WARN_AT`, `MUTE_AT`, `SUSPEND_AT`, `ESCALATE_AT`), durées, réhabilitation (`REHAB_AFTER_DAYS`, `REHAB_STEP`, `REHAB_INTERVAL_MINUTES`) — voir `.env.example` |

## Endpoints

Spécification complète (schémas, codes d'erreur) : [`openapi.yaml`](openapi.yaml) — importable dans Bruno, Postman, Swagger UI, etc.

Erreurs : `{ "error": "CODE", "message": "...", "status": 4xx }`.

### Public
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/health` | Liveness |
| GET | `/openapi.yaml` | Document OpenAPI du service (aussi exposé sur `/api/openapi.yaml`) |

### Joueur (`Authorization: Bearer <JWT Keycloak>`)
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/me` | Mon profil + présence (créé au premier appel) + mon groupe (`group`, `null` si aucun) |
| PATCH | `/api/me` | `displayName, avatarUrl, faction, biography, rpSheet{characterName,story,alignment}` |
| GET | `/api/me/activity?limit=` | Mon historique d'activité |
| GET | `/api/profiles?search=&limit=&entityType=` | Recherche par nom d'affichage (`entityType` ∈ `player\|npc`, défaut : les deux) |
| GET | `/api/profiles/:playerId` | Profil public + statut en ligne |
| GET | `/api/friends` | Amis + présence (statut, localisation) |
| GET | `/api/friends/online` | Amis connectés / en mission |
| GET | `/api/friends/suggestions?limit=` | Joueurs rencontrés récemment, pas encore amis |
| GET | `/api/friends/requests` | `{ incoming, outgoing }` |
| POST | `/api/friends/requests` `{playerId}` | Envoyer une demande (accepte automatiquement une demande inverse en attente) |
| POST | `/api/friends/requests/:id/accept` | Accepter (destinataire) |
| POST | `/api/friends/requests/:id/decline` | Refuser (destinataire) ou annuler (émetteur) |
| DELETE | `/api/friends/:playerId` | Retirer un ami |
| GET | `/api/blocks` | Joueurs bloqués |
| POST | `/api/blocks` `{playerId}` | Bloquer (supprime amitié / demandes, empêche les nouvelles) |
| DELETE | `/api/blocks/:playerId` | Débloquer |
| GET | `/api/me/corporations` | Mes corporations + mon grade dans chacune (vide si aucune) |
| GET | `/api/me/corporation/requests` | Mes invitations et candidatures en attente |
| POST | `/api/me/corporation/requests/:id/accept` | Accepter une invitation |
| POST | `/api/me/corporation/requests/:id/decline` | Refuser une invitation / retirer une candidature |
| GET | `/api/me/groups` | Mon groupe (un joueur appartient au plus à **un** groupe), ou `null` |
| GET | `/api/me/group/invitations` | Mes invitations de groupe en attente |
| POST | `/api/me/group/invitations/:id/accept` | Accepter une invitation de groupe |
| POST | `/api/me/group/invitations/:id/decline` | Refuser une invitation de groupe |
| GET | `/api/me/reputation?limit=` | Mon score, l'historique des variations et mes sanctions actives (accessible même suspendu) |
| GET | `/api/me/sanctions` | Mes sanctions actives (accessible même suspendu) |
| POST | `/api/reports` `{targetType: player\|corporation, targetId, reason, message?}` | Signaler (motifs : `harassment, cheating, griefing, offensive_name, scam, other`) |
| GET | `/api/reports?limit=` | Mes signalements |

Un joueur sous **suspension** ou **ban** actif reçoit `403 SANCTIONED` sur toute l'API joueur sauf `/api/me/reputation` et `/api/me/sanctions`. Un `mute` n'est pas appliqué ici (c'est au chat/serveur de jeu de le lire via l'API interne).

### Corporations (`Authorization: Bearer <JWT Keycloak>`) — un joueur peut appartenir à plusieurs corporations ; une corporation peut être la filiale d'une autre (maison mère)
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/corporations?search=&limit=` | Annuaire (nom/ticker, nombre de membres) |
| POST | `/api/corporations` `{name, ticker, description?, logoUrl?, recruitment?}` | Créer ; le créateur devient le CEO avec le grade CEO (un joueur peut créer/posséder plusieurs corporations) |
| GET | `/api/corporations/:corporationId` | Page publique : corporation, grades, membres + présence, maison mère (`parent`) et filiales (`subsidiaries`) |
| PATCH | `/api/corporations/:corporationId` | `manage_corporation` — nom, ticker, logo, description, `recruitment` ∈ `open\|apply\|closed` |
| DELETE | `/api/corporations/:corporationId` | Dissoudre (CEO) ; les filiales deviennent indépendantes |
| GET | `/api/corporations/:corporationId/subsidiaries` | Filiales directes |
| PUT | `/api/corporations/:corporationId/parent` `{parentId: uuid\|null}` | Rattacher à une maison mère ou détacher (`manage_corporation`, refus des cycles : `409 CORPORATION_CYCLE`) |
| PUT | `/api/corporations/:corporationId/politics` `{politicalEntityId: uuid\|null}` | Rattacher la corporation à une entité politique (**siège fiscal**) ou détacher (`manage_corporation`) |
| POST | `/api/corporations/:corporationId/transfer` `{playerId}` | Transférer le rôle de CEO (CEO) |
| GET | `/api/corporations/:corporationId/activity?limit=` | Journal interne (membres) |
| GET | `/api/corporations/:corporationId/members` | Membres avec grade et présence |
| PATCH | `/api/corporations/:corporationId/members/:playerId` `{rankId}` | Changer le grade (`manage_members`, grades strictement inférieurs au sien) |
| DELETE | `/api/corporations/:corporationId/members/:playerId` | Quitter (soi-même) ou exclure (`manage_members`) |
| GET | `/api/corporations/:corporationId/ranks` | Grades (priorité décroissante) |
| POST | `/api/corporations/:corporationId/ranks` `{name, priority, permissions[], isDefault?}` | Créer un grade (`manage_ranks`) |
| PATCH | `/api/corporations/:corporationId/ranks/:rankId` | Modifier (`manage_ranks` ; le grade CEO n'accepte qu'un renommage) |
| DELETE | `/api/corporations/:corporationId/ranks/:rankId` | Supprimer (membres déplacés vers le grade par défaut) |
| POST | `/api/corporations/:corporationId/join` `{message?}` | Rejoindre directement (`open`) ou candidater (`apply`) |
| POST | `/api/corporations/:corporationId/invitations` `{playerId}` | Inviter (`invite`) |
| GET | `/api/corporations/:corporationId/requests` | Candidatures et invitations en attente (`recruit` ou `invite`) |
| POST | `/api/corporations/:corporationId/requests/:id/accept` | Accepter une candidature (`recruit`) |
| POST | `/api/corporations/:corporationId/requests/:id/decline` | Refuser une candidature (`recruit`) ou retirer une invitation (`invite`) |

Permissions de grade : `manage_corporation`, `manage_ranks`, `manage_members`, `invite`, `recruit`. Le grade CEO (unique, indélébile) les a toutes. Grades créés par défaut : CEO (100), Director (50 : invite, recruit, manage_members), Member (0, grade par défaut). Une candidature croisée avec une invitation est acceptée automatiquement.

**Multi-appartenance & hiérarchie** : un joueur/PNJ peut être membre de plusieurs corporations (aucune contrainte d'unicité par joueur ; les doublons sont interdits par corporation). Une corporation peut être rattachée à une **maison mère** via `parentId` : la page publique expose `parent` (référence) et `subsidiaries` (filiales directes), `GET /api/corporations/:id/subsidiaries` liste les filiales et `PUT /api/corporations/:id/parent` rattache/détache (les cycles sont refusés). Dissoudre une maison mère laisse ses filiales indépendantes (`ON DELETE SET NULL`).

### Groupes (`Authorization: Bearer <JWT Keycloak>`) — groupes **temporaires** pour partager des missions ; un joueur appartient au plus à **un** groupe
| Méthode | Route | Description |
|---|---|---|
| POST | `/api/groups` `{name, description?, maxMembers?}` | Créer un groupe ; le créateur devient owner et premier membre (`409 ALREADY_IN_GROUP` si déjà dans un groupe) |
| GET | `/api/groups/:groupId` | Résumé : groupe + `memberCount` (membres uniquement) |
| PATCH | `/api/groups/:groupId` | `name, description, maxMembers` (owner) |
| DELETE | `/api/groups/:groupId` | Dissoudre (owner ; `members` et invitations supprimés en cascade) |
| GET | `/api/groups/:groupId/members` | Membres avec profil et présence (owner d'abord, puis ancienneté) |
| DELETE | `/api/groups/:groupId/members/:playerId` | Exclure un membre (owner ; l'owner ne peut pas être exclu) |
| POST | `/api/groups/:groupId/leave` | Quitter le groupe — si l'owner quitte, le **membre le plus ancien devient owner** ; s'il ne reste personne, le groupe est supprimé |
| POST | `/api/groups/:groupId/invitations` `{playerId}` | Inviter un joueur (owner ; `409 ALREADY_IN_GROUP` / `GROUP_FULL` / invitation en attente) |

**Règles** : pas d'expiration automatique (« temporaire » = dissolution manuelle par l'owner ou auto à vide) ; **1 groupe par joueur** garanti par contrainte unique (`group_members.player_id`) ; cap `maxMembers` 2..100 (défaut 10), compté owner inclus. L'invitation est proposée au destinataire via `/api/me/group/invitations` (accept/refuse). Le service Mission vérifie l'appartenance via l'API interne.

### Politique (`Authorization: Bearer <JWT Keycloak>`)
Catégorie sociale hiérarchique : **commune** (villages/villes, avec maire et conseil) → **agglomération** → **département** → **région** → **pays** → **fédération** (niveau le plus haut, optionnel — un pays peut rester indépendant). Une entité politique a des **offices** (maire, président, conseiller…), des **membres** (joueurs **et** PNJ) et un **office de tête**. Un profil (joueur ou PNJ) peut occuper n'importe quel office, y compris la tête.

| Méthode | Route | Description |
|---|---|---|
| GET | `/api/politics?search=&type=&limit=` | Annuaire (nom, nombre de membres, filtre de niveau `type`) |
| POST | `/api/politics` `{type, name, description?, bannerUrl?}` | Créer ; le créateur devient la tête (offices par défaut semés selon le niveau) |
| GET | `/api/politics/:entityId` | Page publique : offices, membres + présence, entité parente et enfants |
| PATCH | `/api/politics/:entityId` | `manage_entity` — nom, description, bannière |
| DELETE | `/api/politics/:entityId` | Dissoudre (tête) ; les enfants deviennent indépendants |
| GET | `/api/politics/:entityId/children` | Entités de niveau inférieur directement rattachées |
| PUT | `/api/politics/:entityId/parent` `{parentId: uuid\|null}` | Rattacher à une entité de **niveau strictement supérieur** ou détacher (`manage_hierarchy`, refus des cycles `409 POLITICAL_CYCLE`, niveau invalide `400 INVALID_PARENT_LEVEL`) |
| POST | `/api/politics/:entityId/transfer` `{playerId}` | Transférer la tête (tête) |
| GET | `/api/politics/:entityId/activity?limit=` | Journal interne (membres) |
| GET | `/api/politics/:entityId/members` | Membres avec office et présence |
| POST | `/api/politics/:entityId/members` `{playerId, officeId?}` | Nommer un membre (`manage_members`) |
| PATCH | `/api/politics/:entityId/members/:playerId` `{officeId}` | Changer d'office (`manage_members`, offices strictement inférieurs au sien) |
| DELETE | `/api/politics/:entityId/members/:playerId` | Quitter (soi-même) ou révoquer (`manage_members`) |
| GET | `/api/politics/:entityId/offices` | Offices (priorité décroissante) |
| POST | `/api/politics/:entityId/offices` `{name, priority, permissions[], isDefault?}` | Créer un office (`manage_offices`) |
| PATCH | `/api/politics/:entityId/offices/:officeId` | Modifier (`manage_offices` ; l'office de tête n'accepte qu'un renommage) |
| DELETE | `/api/politics/:entityId/offices/:officeId` | Supprimer (membres déplacés vers l'office par défaut) |

Permissions d'office : `manage_entity`, `manage_offices`, `manage_members`, `manage_hierarchy`, `manage_treasury` (dépenser/configurer la trésorerie), `issue_currency` (créer de la monnaie, pays/fédération). L'office de tête (unique, indélébile) les a toutes. Offices semés : `commune` → Maire (tête) / Deputy / Councilor / Citizen (défaut) ; niveaux intermédiaires → President (tête) / Vice President / Conseiller / Resident ; `country` → Head of State / Minister / Deputy / Citizen ; `federation` → President / Representative / Citizen.

La **trésorerie**, les **taxes** et l'**émission monétaire** sont gérées par le service [`economie`](../economie/Readme.md) (l'entité politique y possède un compte `political`) : le serveur de jeu vérifie le niveau et l'office ici, puis appelle `economie`.

`GET /api/me` (et `GET /api/me/politics`) expose les appartenances politiques du joueur ; `GET /api/profiles/:playerId` les expose aussi (`politics: [{ id, type, name }]`).

### Modération (`Authorization: Bearer` avec rôle Keycloak `moderator` < `admin` < `supervisor`)
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/admin/stats` | Analyse communautaire : joueurs/en ligne, corporations (top 5), signalements par statut, sanctions actives, activité 24h, plus signalés, réputations les plus basses |
| GET | `/api/admin/log?limit=` | Journal d'audit des actions de modération |
| GET | `/api/admin/reports?status=&escalation=&targetPlayerId=&limit=` | File des signalements |
| GET | `/api/admin/reports/:id` | Détail |
| PATCH | `/api/admin/reports/:id` `{status: reviewing\|resolved\|dismissed, note?}` | `resolved` = signalement confirmé (pénalité `UPHELD_REPORT_PENALTY`), `dismissed` rembourse la pénalité initiale |
| POST | `/api/admin/reports/:id/escalate` | Escalade d'un niveau (moderator → admin → supervisor) |
| GET | `/api/admin/players/:playerId` | Fiche : profil, historique de réputation, sanctions, signalements reçus, activité |
| POST | `/api/admin/players/:playerId/reputation` `{delta, reason}` | Ajustement manuel (**admin**) |
| POST | `/api/admin/players/:playerId/sanctions` `{type, reason, durationHours?}` | `warning`/`mute` : moderator ; `suspension`/`ban` : **admin** |
| GET | `/api/admin/sanctions?playerId=&active=&limit=` | Liste des sanctions |
| DELETE | `/api/admin/sanctions/:id` | Révoquer |

**Réputation** : chaque variation est un événement (`source` ∈ `game, block, report, sanction, moderation, rehabilitation`). Blocage = −`BLOCK_PENALTY` (rendu au déblocage), signalement = −`REPORT_PENALTY`, signalement confirmé = −`UPHELD_REPORT_PENALTY`. Sous les seuils, sanction automatique si aucune du même type n'est active : avertissement (`WARN_AT`), mute `MUTE_HOURS` (`MUTE_AT`), suspension `SUSPEND_HOURS` (`SUSPEND_AT`) ; sous `ESCALATE_AT` un signalement système est ouvert au niveau `admin`. **Réhabilitation** : les joueurs sous 0 sans événement depuis `REHAB_AFTER_DAYS` jours regagnent `REHAB_STEP` point(s) à chaque passe (planifiée en process toutes les `REHAB_INTERVAL_MINUTES` minutes, ou déclenchée via l'API interne).

### Interne — serveur de jeu (token Keycloak de service + rôle de capacité)
| Méthode | Route | Rôle requis | Description |
|---|---|---|---|
| PUT | `/api/internal/players/:playerId` `{displayName}` | `social:profile:write` | Créer le profil au login (nom existant conservé) |
| GET | `/api/internal/players/:playerId` | `social:profile:read` | Profil complet (**réputation** inclus) ou `null` — utilisé par Mission pour les prérequis `min_reputation` |
| PUT | `/api/internal/players/:playerId/npc` `{displayName, avatarUrl?, faction?, biography?, role?}` | `social:profile:write` | Créer/mettre à jour un profil **PNJ** (nom déjà pris → 409, jamais renommé) |
| PUT | `/api/internal/players/:playerId/presence` `{status, location?}` | `social:profile:write` | `status` ∈ `online\|mission\|offline`, `location{system,scene,position{x,y,z}}` |
| GET | `/api/internal/players/:playerId/presence` | `social:profile:read` | Statut + localisation (défaut hors ligne / `location: null`) — lu par Mission pour filtrer les missions zonées |
| POST | `/api/internal/players/:playerId/stats` | `social:player:write` | `playtimeSecondsDelta, role, reputationDelta, reputationReason` (la réputation passe par le système d'événements) |
| GET | `/api/internal/players/:playerId/sanctions` | `social:sanctions:read` | Sanctions actives (pour appliquer mute/ban côté jeu) |
| POST | `/api/internal/reputation/rehabilitate` | `social:reputation:write` | Lancer une passe de réhabilitation |
| POST | `/api/internal/players/:playerId/activity` `{type, details?}` | `social:player:write` | Ajouter une entrée d'activité |
| GET | `/api/internal/players/:playerId/corporation?corporationId=` | `social:corporation:read` | Avec `corporationId` : cette adhésion ou `null` ; sans : la liste des adhésions d'un joueur |
| GET | `/api/internal/players/:playerId/group?groupId=` | `social:group:read` | Avec `groupId` : cette adhésion ou `null` ; sans : le groupe unique du joueur ou `null` — `{group, member}` |
| GET | `/api/internal/groups/:groupId` | `social:group:read` | Résumé du groupe (`memberCount`) ou `null` si inexistant — utilisé par Mission pour valider un partage |
| GET | `/api/internal/players?search=&playerIds=&limit=` | `social:profile:read` | Résoudre des profils par pseudo (sous-chaîne) et/ou ids explicites — `{playerId, displayName, entityType}` (pour les services qui ne stockent que des UUID) |
| PUT | `/api/internal/players/:playerId/corporation` `{corporationId, rankId?}` | `social:corporation:write` | Ajouter un **PNJ** à une corporation (grade par défaut si omis) |
| DELETE | `/api/internal/players/:playerId/corporation?corporationId=` | `social:corporation:write` | Retirer un **PNJ** d'une corporation précise |
| POST | `/api/internal/corporations` `{ceoId, name, ticker, description?, logoUrl?, recruitment?}` | `social:corporation:write` | Créer une corporation avec un CEO explicite (joueur ou **PNJ**) |
| PATCH | `/api/internal/corporations/:corporationId` | `social:corporation:write` | Modifier (agit comme le CEO courant) |
| DELETE | `/api/internal/corporations/:corporationId` | `social:corporation:write` | Dissoudre (agit comme le CEO courant) |
| POST | `/api/internal/corporations/:corporationId/transfer` `{playerId}` | `social:corporation:write` | Transférer le CEO (agit comme le CEO courant) |
| PATCH | `/api/internal/corporations/:corporationId/members/:playerId` `{rankId}` | `social:corporation:write` | Changer le grade d'un membre (agit comme le CEO) |
| DELETE | `/api/internal/corporations/:corporationId/members/:playerId` | `social:corporation:write` | Retirer un membre (agit comme le CEO) |
| GET | `/api/internal/corporations/:corporationId/politics` | `social:corporation:read` | Siège fiscal de la corporation (`politicalEntityId`) — pour le miroir vers economie |
| POST | `/api/internal/politics` `{headId, type, name, description?, bannerUrl?}` | `social:politics:write` | Créer une entité politique avec un chef explicite (joueur ou **PNJ**) |
| PATCH | `/api/internal/politics/:entityId` | `social:politics:write` | Modifier (agit comme le chef courant) |
| DELETE | `/api/internal/politics/:entityId` | `social:politics:write` | Dissoudre (agit comme le chef courant) |
| POST | `/api/internal/politics/:entityId/transfer` `{playerId}` | `social:politics:write` | Transférer la tête (agit comme le chef courant) |
| PUT | `/api/internal/players/:playerId/politics` `{entityId, officeId?}` | `social:politics:write` | Ajouter un **PNJ** à une entité politique (office par défaut si omis) |
| DELETE | `/api/internal/players/:playerId/politics?entityId=` | `social:politics:write` | Retirer un **PNJ** d'une entité politique |
| GET | `/api/internal/players/:playerId/politics?entityId=` | `social:politics:read` | Avec `entityId` : cette adhésion ou `null` ; sans : la liste des adhésions politiques |
| POST | `/api/internal/encounters` `{playerId, otherPlayerId}` | `social:reputation:write` | Enregistrer une rencontre (alimente les suggestions) |

**Auth de service** : garde monté sur le préfixe (`app.use('/api/internal', serviceAuth, …)`) ; `azp` ∈ `INTERNAL_SERVICE_CLIENTS`, `aud` = `social-api`, puis rôle de capacité sinon `403 SERVICE_FORBIDDEN`/`FORBIDDEN`. Un token joueur ne peut pas porter un `azp` de service : c'est la garantie de non-contournement.

Les **PNJ** sont des profils `entityType: "npc"` (id UUID attribué par le serveur de jeu) : visibles dans la recherche (`?entityType=npc`), amiables, présents dans les corporations comme membres avec un grade, et **exclus** de la réputation, des sanctions et des signalements (réponse `400 NPC_NOT_APPLICABLE`). Un PNJ **peut** être le CEO d'une corporation (via l'API interne) : le jeu peut ainsi créer des corporations entièrement PNJ.

## Structure

```
src/
  index.ts          bootstrap Express, migrations, listen
  config/env.ts     variables d'environnement
  db/schema/        tables drizzle (profiles, presence, friendships, blocks, encounters, activity, corporations, groups, politics, moderation)
  db/connection.ts  pool pg + drizzle ; db/migrate.ts applique ./drizzle
  middleware/       auth (JWT joueur / service-account + rôles de capacité), sanctions, validate (zod), errorHandler
  routes/           un routeur par ressource, schémas zod dans routes/schemas.ts
  services/         logique métier (une fonction exportée par cas d'usage)
```

Déploiement : `docker/Dockerfile` (image standalone, migrations au démarrage), workflows `.github/workflows/build-*-social.yaml`.

## Feuille de route

Ce service est dédié à la partie sociale du jeu ; les features ci-dessous sont traitées pas à pas.

### Profils Joueurs
- [x] Création et gestion du profil (nom, avatar, faction, biographie)
- [x] Profils NPC côté serveur (`entityType: npc`, gestion via l'API interne, visibles et amiables, exclus de la modération)
- [x] Statistiques personnelles (temps de jeu, réputation (joueur), rôle, etc.)
- [x] Historique d'activité
- [x] Réputation (joueur) dynamique selon les interactions et signalements
- [x] Fiche RP optionnelle (identité de personnage, histoire, alignement)

### Relations Sociales
- [x] Liste d'amis et gestion des invitations
- [x] Statut en ligne (connecté / mission / hors ligne)
- [x] Localisation des amis dans l'univers persistant
- [~] Invitations contextuelles (groupe, corporation, mission) — corporation et **groupe** faits, invitations de mission à venir
- [x] Système de recommandations ("joueurs rencontrés récemment")

### Corporations
- [x] Création et gestion de corporation (nom, ticker, logo, description)
- [x] Système de grades et permissions internes
- [x] Page publique de corporation avec présentation et statistiques
- [x] Recrutement et gestion des membres
- [x] Multi-appartenance (un joueur peut rejoindre plusieurs corporations)
- [x] Hiérarchie maison mère / filiales (`parentId`, rattachement, anti-cycle)
- [ ] Relations diplomatiques (alliances, trêves, guerres)
- [x] Journal d'activité interne (actions, promotions, missions)
- [x] Têtes PNJ et corporations entièrement PNJ (CEO PNJ via l'API interne)
- [ ] Système de territoires (stations, flottes, zones contrôlées)
- [ ] Classements et influence inter-corporations

### Politique
- [x] Entités politiques hiérarchiques : commune (village/ville) → agglomération → département → région → pays → fédération (optionnelle)
- [x] Offices (maire, président, conseiller…), permissions et office de tête
- [x] Membres joueurs **et** PNJ, nomination par la tête, API interne pour les PNJ
- [x] Hiérarchie parent/enfants avec contrôle de niveau et anti-cycle
- [x] Appartenances politiques exposées sur le profil (`/api/me`, `/api/profiles/:id`)
- [x] Siège fiscal des corporations (`politicalEntityId`) et permissions `manage_treasury` / `issue_currency`
- [ ] Trésorerie, taxes et émission monétaire (implémentées dans le service [`economie`](../economie/Readme.md))
- [ ] Élections (candidatures et vote des membres) pour choisir la tête et les offices
- [ ] Adhésion ouverte (open/apply/closed) et candidatures si ouverture aux joueurs
- [ ] Relations entre entités (alliances, traités, guerres)

### Réputation joueur & Modération
- [x] Système de réputation global pour chaque joueur
- [x] Signalement d'un joueur ou d'une corporation avec motif
- [x] Impact des blocages/ignorances sur la réputation
- [x] Sanctions automatiques selon le score de réputation
- [x] Escalade automatique vers des instances supérieures
- [x] Historique de réputation et mécanisme de réhabilitation

### API & Intégration
- [x] API interne connectée au serveur du jeu (mise à jour régulière)
- [x] Auth service-à-service par comptes de service Keycloak et rôles de capacité (fin des secrets partagés)
- [~] API publique sécurisée (OAuth2, clés d'accès) — JWT Keycloak en place, clés d'accès tierces à venir
- [ ] Webhooks d'événements (nouvelle corporation, changement de réputation, etc.)
- [ ] Support des outils externes (bots, extensions, overlays)

### Administration & Modération
- [x] Rôles spécifiques de modération (modérateurs, administrateurs, superviseurs)
- [x] Tableau de bord de gestion des signalements et réputations
- [~] Outils d'analyse communautaire (activité, interactions, corporations influentes) — `GET /api/admin/stats` (base), à enrichir
