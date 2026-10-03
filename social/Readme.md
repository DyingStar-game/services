# Service social

API sociale de DyingStar : profils joueurs, amis, présence, et à terme corporations, réputation et modération.
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
| GET | `/api/me` | Mon profil + présence (créé au premier appel) |
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
| PUT | `/api/internal/players/:playerId/npc` `{displayName, avatarUrl?, faction?, biography?, role?}` | `social:profile:write` | Créer/mettre à jour un profil **PNJ** (nom déjà pris → 409, jamais renommé) |
| PUT | `/api/internal/players/:playerId/presence` `{status, location?}` | `social:profile:write` | `status` ∈ `online\|mission\|offline`, `location{system,scene,position{x,y,z}}` |
| POST | `/api/internal/players/:playerId/stats` | `social:player:write` | `playtimeSecondsDelta, role, reputationDelta, reputationReason` (la réputation passe par le système d'événements) |
| GET | `/api/internal/players/:playerId/sanctions` | `social:sanctions:read` | Sanctions actives (pour appliquer mute/ban côté jeu) |
| POST | `/api/internal/reputation/rehabilitate` | `social:reputation:write` | Lancer une passe de réhabilitation |
| POST | `/api/internal/players/:playerId/activity` `{type, details?}` | `social:player:write` | Ajouter une entrée d'activité |
| GET | `/api/internal/players/:playerId/corporation?corporationId=` | `social:corporation:read` | Avec `corporationId` : cette adhésion ou `null` ; sans : la liste des adhésions d'un joueur |
| GET | `/api/internal/players?search=&playerIds=&limit=` | `social:profile:read` | Résoudre des profils par pseudo (sous-chaîne) et/ou ids explicites — `{playerId, displayName, entityType}` (pour les services qui ne stockent que des UUID) |
| PUT | `/api/internal/players/:playerId/corporation` `{corporationId, rankId?}` | `social:corporation:write` | Ajouter un **PNJ** à une corporation (grade par défaut si omis) |
| DELETE | `/api/internal/players/:playerId/corporation?corporationId=` | `social:corporation:write` | Retirer un **PNJ** d'une corporation précise |
| POST | `/api/internal/encounters` `{playerId, otherPlayerId}` | `social:reputation:write` | Enregistrer une rencontre (alimente les suggestions) |

**Auth de service** : garde monté sur le préfixe (`app.use('/api/internal', serviceAuth, …)`) ; `azp` ∈ `INTERNAL_SERVICE_CLIENTS`, `aud` = `social-api`, puis rôle de capacité sinon `403 SERVICE_FORBIDDEN`/`FORBIDDEN`. Un token joueur ne peut pas porter un `azp` de service : c'est la garantie de non-contournement.

Les **PNJ** sont des profils `entityType: "npc"` (id UUID attribué par le serveur de jeu) : visibles dans la recherche (`?entityType=npc`), amiables et présents dans les corporations comme membres avec un grade, mais **exclus** de la réputation, des sanctions et des signalements (réponse `400 NPC_NOT_APPLICABLE`). Un PNJ ne peut jamais être le CEO d'une corporation.

## Structure

```
src/
  index.ts          bootstrap Express, migrations, listen
  config/env.ts     variables d'environnement
  db/schema/        tables drizzle (profiles, presence, friendships, blocks, encounters, activity, corporations, moderation)
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
- [~] Invitations contextuelles (groupe, corporation, mission) — corporation faite, groupe/mission à venir
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
- [ ] Système de territoires (stations, flottes, zones contrôlées)
- [ ] Classements et influence inter-corporations

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
