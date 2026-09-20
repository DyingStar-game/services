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
| `AUTH_DEV_BYPASS` | Accepter `X-Player-Id` sans JWT (dev uniquement) |

## Endpoints

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

### Interne — serveur de jeu (`X-Internal-Key`)
| Méthode | Route | Description |
|---|---|---|
| PUT | `/api/internal/players/:playerId` `{displayName}` | Créer le profil au login (nom existant conservé) |
| PUT | `/api/internal/players/:playerId/presence` `{status, location?}` | `status` ∈ `online\|mission\|offline`, `location{system,scene,position{x,y,z}}` |
| POST | `/api/internal/players/:playerId/stats` | `playtimeSecondsDelta, reputationDelta, level, role` |
| POST | `/api/internal/players/:playerId/activity` `{type, details?}` | Ajouter une entrée d'activité |
| POST | `/api/internal/encounters` `{playerId, otherPlayerId}` | Enregistrer une rencontre (alimente les suggestions) |

## Structure

```
src/
  index.ts          bootstrap Express, migrations, listen
  config/env.ts     variables d'environnement
  db/schema/        tables drizzle (profiles, presence, friendships, blocks, encounters, activity)
  db/connection.ts  pool pg + drizzle ; db/migrate.ts applique ./drizzle
  middleware/       auth (JWT / clé interne), validate (zod), errorHandler
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
- [ ] Réputation (joueur) dynamique selon les interactions et signalements
- [x] Fiche RP optionnelle (identité de personnage, histoire, alignement)

### Relations Sociales
- [x] Liste d'amis et gestion des invitations
- [x] Statut en ligne (connecté / mission / hors ligne)
- [x] Localisation des amis dans l'univers persistant
- [ ] Invitations contextuelles (groupe, guilde, mission)
- [x] Système de recommandations ("joueurs rencontrés récemment")

### Guildes
- [ ] Création et gestion de guilde (nom, logo, description, tag)
- [ ] Système de grades et permissions internes
- [ ] Page publique de guilde avec présentation et statistiques
- [ ] Recrutement et gestion des membres
- [ ] Relations diplomatiques (alliances, trêves, guerres)
- [ ] Journal d'activité interne (actions, promotions, missions)
- [ ] Système de territoires (stations, flottes, zones contrôlées)
- [ ] Classements et influence inter-guildes

### Réputation joueur & Modération
- [ ] Système de réputation global pour chaque joueur
- [ ] Signalement d'un joueur ou d'une guilde avec motif
- [ ] Impact des blocages/ignorances sur la réputation
- [ ] Sanctions automatiques selon le score de réputation
- [ ] Escalade automatique vers des instances supérieures
- [ ] Historique de réputation et mécanisme de réhabilitation

### API & Intégration
- [x] API interne connectée au serveur du jeu (mise à jour régulière)
- [~] API publique sécurisée (OAuth2, clés d'accès) — JWT Keycloak en place, clés d'accès tierces à venir
- [ ] Webhooks d'événements (nouvelle guilde, changement de réputation, etc.)
- [ ] Support des outils externes (bots, extensions, overlays)

### Administration & Modération
- [ ] Rôles spécifiques de modération (modérateurs, administrateurs, superviseurs)
- [ ] Tableau de bord de gestion des signalements et réputations
- [ ] Outils d'analyse communautaire (activité, interactions, guildes influentes)
