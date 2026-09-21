# Service social

API sociale de DyingStar : profils joueurs, amis, présence, et à terme guildes, réputation et modération.
Stack : Node 22, TypeScript, Express 4, drizzle-orm + PostgreSQL, JWT Keycloak validé via JWKS (`jose`).

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
curl localhost:3000/api/me -H "X-Player-Id: 11111111-1111-4111-8111-111111111111" -H "X-Player-Name: alice"
```

## Variables d'environnement

| Variable | Rôle |
|---|---|
| `PORT`, `NODE_ENV`, `CORS_ORIGIN` | HTTP |
| `DATABASE_URL` | PostgreSQL (obligatoire) |
| `OIDC_ISSUER`, `OIDC_JWKS_URL`, `OIDC_AUDIENCE` | Validation des JWT Keycloak (`player_id` = claim `sub`) |
| `INTERNAL_API_KEY` | Secret attendu dans `X-Internal-Key` sur `/api/internal/*` |
| `AUTH_DEV_BYPASS` | Accepter `X-Player-Id` (+ `X-Player-Name`, `X-Player-Roles`) sans JWT (dev uniquement) |
| `REPUTATION_*` | Pénalités (blocage, signalement, signalement confirmé), seuils de sanctions automatiques (`WARN_AT`, `MUTE_AT`, `SUSPEND_AT`, `ESCALATE_AT`), durées, réhabilitation (`REHAB_AFTER_DAYS`, `REHAB_STEP`, `REHAB_INTERVAL_MINUTES`) — voir `.env.example` |

## Endpoints

Spécification complète (schémas, codes d'erreur) : [`openapi.yaml`](openapi.yaml) — importable dans Bruno, Postman, Swagger UI, etc.

Erreurs : `{ "error": "CODE", "message": "...", "status": 4xx }`.

### Public
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/health` | Liveness |

### Joueur (`Authorization: Bearer <JWT Keycloak>`)
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/me` | Mon profil + présence (créé au premier appel) |
| PATCH | `/api/me` | `displayName, avatarUrl, faction, biography, rpSheet{characterName,story,alignment}` |
| GET | `/api/me/activity?limit=` | Mon historique d'activité |
| GET | `/api/profiles?search=&limit=` | Recherche par nom d'affichage |
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
| GET | `/api/me/guild` | Ma guilde + mon grade (`null` si aucune) |
| GET | `/api/me/guild/requests` | Mes invitations et candidatures en attente |
| POST | `/api/me/guild/requests/:id/accept` | Accepter une invitation |
| POST | `/api/me/guild/requests/:id/decline` | Refuser une invitation / retirer une candidature |
| GET | `/api/me/reputation?limit=` | Mon score, l'historique des variations et mes sanctions actives (accessible même suspendu) |
| GET | `/api/me/sanctions` | Mes sanctions actives (accessible même suspendu) |
| POST | `/api/reports` `{targetType: player\|guild, targetId, reason, message?}` | Signaler (motifs : `harassment, cheating, griefing, offensive_name, scam, other`) |
| GET | `/api/reports?limit=` | Mes signalements |

Un joueur sous **suspension** ou **ban** actif reçoit `403 SANCTIONED` sur toute l'API joueur sauf `/api/me/reputation` et `/api/me/sanctions`. Un `mute` n'est pas appliqué ici (c'est au chat/serveur de jeu de le lire via l'API interne).

### Guildes (`Authorization: Bearer <JWT Keycloak>`) — un joueur appartient à une guilde max
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/guilds?search=&limit=` | Annuaire (nom/tag, nombre de membres) |
| POST | `/api/guilds` `{name, tag, description?, logoUrl?, recruitment?}` | Créer ; le créateur devient propriétaire avec le grade leader |
| GET | `/api/guilds/:guildId` | Page publique : guilde, grades, membres + présence |
| PATCH | `/api/guilds/:guildId` | `manage_guild` — nom, tag, logo, description, `recruitment` ∈ `open\|apply\|closed` |
| DELETE | `/api/guilds/:guildId` | Dissoudre (propriétaire) |
| POST | `/api/guilds/:guildId/transfer` `{playerId}` | Transférer la propriété (propriétaire) |
| GET | `/api/guilds/:guildId/activity?limit=` | Journal interne (membres) |
| GET | `/api/guilds/:guildId/members` | Membres avec grade et présence |
| PATCH | `/api/guilds/:guildId/members/:playerId` `{rankId}` | Changer le grade (`manage_members`, grades strictement inférieurs au sien) |
| DELETE | `/api/guilds/:guildId/members/:playerId` | Quitter (soi-même) ou exclure (`manage_members`) |
| GET | `/api/guilds/:guildId/ranks` | Grades (priorité décroissante) |
| POST | `/api/guilds/:guildId/ranks` `{name, priority, permissions[], isDefault?}` | Créer un grade (`manage_ranks`) |
| PATCH | `/api/guilds/:guildId/ranks/:rankId` | Modifier (`manage_ranks` ; le grade leader n'accepte qu'un renommage) |
| DELETE | `/api/guilds/:guildId/ranks/:rankId` | Supprimer (membres déplacés vers le grade par défaut) |
| POST | `/api/guilds/:guildId/join` `{message?}` | Rejoindre directement (`open`) ou candidater (`apply`) |
| POST | `/api/guilds/:guildId/invitations` `{playerId}` | Inviter (`invite`) |
| GET | `/api/guilds/:guildId/requests` | Candidatures et invitations en attente (`recruit` ou `invite`) |
| POST | `/api/guilds/:guildId/requests/:id/accept` | Accepter une candidature (`recruit`) |
| POST | `/api/guilds/:guildId/requests/:id/decline` | Refuser une candidature (`recruit`) ou retirer une invitation (`invite`) |

Permissions de grade : `manage_guild`, `manage_ranks`, `manage_members`, `invite`, `recruit`. Le grade leader (unique, indélébile) les a toutes. Grades créés par défaut : Leader (100), Officer (50 : invite, recruit, manage_members), Member (0, grade par défaut). Une candidature croisée avec une invitation est acceptée automatiquement.

### Modération (`Authorization: Bearer` avec rôle Keycloak `moderator` < `admin` < `supervisor`)
| Méthode | Route | Description |
|---|---|---|
| GET | `/api/admin/stats` | Analyse communautaire : joueurs/en ligne, guildes (top 5), signalements par statut, sanctions actives, activité 24h, plus signalés, réputations les plus basses |
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

### Interne — serveur de jeu (`X-Internal-Key`)
| Méthode | Route | Description |
|---|---|---|
| PUT | `/api/internal/players/:playerId` `{displayName}` | Créer le profil au login (nom existant conservé) |
| PUT | `/api/internal/players/:playerId/presence` `{status, location?}` | `status` ∈ `online\|mission\|offline`, `location{system,scene,position{x,y,z}}` |
| POST | `/api/internal/players/:playerId/stats` | `playtimeSecondsDelta, level, role, reputationDelta, reputationReason` (la réputation passe par le système d'événements) |
| GET | `/api/internal/players/:playerId/sanctions` | Sanctions actives (pour appliquer mute/ban côté jeu) |
| POST | `/api/internal/reputation/rehabilitate` | Lancer une passe de réhabilitation |
| POST | `/api/internal/players/:playerId/activity` `{type, details?}` | Ajouter une entrée d'activité |
| GET | `/api/internal/players/:playerId/guild` | Guilde et grade d'un joueur (`null` si aucune) |
| POST | `/api/internal/encounters` `{playerId, otherPlayerId}` | Enregistrer une rencontre (alimente les suggestions) |

## Structure

```
src/
  index.ts          bootstrap Express, migrations, listen
  config/env.ts     variables d'environnement
  db/schema/        tables drizzle (profiles, presence, friendships, blocks, encounters, activity, guilds, moderation)
  db/connection.ts  pool pg + drizzle ; db/migrate.ts applique ./drizzle
  middleware/       auth (JWT / clé interne / rôles), sanctions, validate (zod), errorHandler
  routes/           un routeur par ressource, schémas zod dans routes/schemas.ts
  services/         logique métier (une fonction exportée par cas d'usage)
```

Déploiement : `docker/Dockerfile` (image standalone, migrations au démarrage), workflows `.github/workflows/build-*-social.yaml`.

## Feuille de route

Ce service est dédié à la partie sociale du jeu ; les features ci-dessous sont traitées pas à pas.

### Profils Joueurs
- [x] Création et gestion du profil (nom, avatar, faction, biographie)
- [x] Statistiques personnelles (temps de jeu, niveau, réputation (joueur), rôle, etc.)
- [x] Historique d'activité
- [x] Réputation (joueur) dynamique selon les interactions et signalements
- [x] Fiche RP optionnelle (identité de personnage, histoire, alignement)

### Relations Sociales
- [x] Liste d'amis et gestion des invitations
- [x] Statut en ligne (connecté / mission / hors ligne)
- [x] Localisation des amis dans l'univers persistant
- [~] Invitations contextuelles (groupe, guilde, mission) — guilde faite, groupe/mission à venir
- [x] Système de recommandations ("joueurs rencontrés récemment")

### Guildes
- [x] Création et gestion de guilde (nom, logo, description, tag)
- [x] Système de grades et permissions internes
- [x] Page publique de guilde avec présentation et statistiques
- [x] Recrutement et gestion des membres
- [ ] Relations diplomatiques (alliances, trêves, guerres)
- [x] Journal d'activité interne (actions, promotions, missions)
- [ ] Système de territoires (stations, flottes, zones contrôlées)
- [ ] Classements et influence inter-guildes

### Réputation joueur & Modération
- [x] Système de réputation global pour chaque joueur
- [x] Signalement d'un joueur ou d'une guilde avec motif
- [x] Impact des blocages/ignorances sur la réputation
- [x] Sanctions automatiques selon le score de réputation
- [x] Escalade automatique vers des instances supérieures
- [x] Historique de réputation et mécanisme de réhabilitation

### API & Intégration
- [x] API interne connectée au serveur du jeu (mise à jour régulière)
- [~] API publique sécurisée (OAuth2, clés d'accès) — JWT Keycloak en place, clés d'accès tierces à venir
- [ ] Webhooks d'événements (nouvelle guilde, changement de réputation, etc.)
- [ ] Support des outils externes (bots, extensions, overlays)

### Administration & Modération
- [x] Rôles spécifiques de modération (modérateurs, administrateurs, superviseurs)
- [x] Tableau de bord de gestion des signalements et réputations
- [~] Outils d'analyse communautaire (activité, interactions, guildes influentes) — `GET /api/admin/stats` (base), à enrichir
