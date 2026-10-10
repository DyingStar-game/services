# Position du joueur : lire depuis Persistence au lieu de la dupliquer

> **Statut : étude uniquement — rien n'est fait, rien n'est planifié.**
> Note d'architecture du 2026-10-10. Action trop lourde pour le moment
> (chantier Persistence : auth + endpoint batch + lecture cache-first).

## Constat : la position est stockée deux fois

| | Persistence | Social (`player_presence.location`) |
|---|---|---|
| Forme | `position` (Vec3 JSON) + `scenename` + `parent_id` (colonnes plates, `persistence/src/db/connection.rs:45`) | `{system, scene, position{x,y,z}}` jsonb (`src/db/schema/presence.ts:13-24`) |
| Encodage scène | chemin fichier `scenes/systems/tarsis_1/planet.tscn` | hiérarchie `tarsis_1/new-paris` — **convention différente** |
| Écriture | à chaque mouvement (merge WS, `persistence/src/websocket/handlers.rs:94`) | heartbeat batch ~1 req/min, DB seulement aux changements de statut, sinon Valkey `EX 90` |
| Sémantique | état du monde, ligne qui persiste après déconnexion | `status` online/mission/offline + TTL 90 s → `offline` auto |
| Lecteurs | serveur de jeu uniquement | friends / corps / groupe / politique / **mission (zones)** |

Le doublon actuel coûte peu (un pipeline `SET EX 90` par minute, zéro écriture PG
par heartbeat), mais c'est deux vérités pour « où est le joueur ».

## Proposition retenue (à étudier plus tard)

- **Status reste dans Social** : c'est la vraie nature du service (liveness, TTL,
  détection de déconnexion automatique). Persistence n'a aucune notion d'online.
- **Position lue depuis Persistence** par ceux qui en ont besoin (zones de mission,
  partage de position aux amis/groupe/corpo) — Persistence est déjà la source
  fraîche (écriture temps réel via WS, cache DualCache + flush Scylla toutes les 60 s).
- **Social reste le point de join** : il expose toujours `location` fusionnée
  (status Valkey + position Persistence) pour ne pas changer les contrats de Mission,
  friends, corps, groupes, politique — et pour appliquer la règle
  « `location` affichée seulement si `status != offline` » au seul endroit qui connaît
  le statut.
- Bénéfice : suppression de l'écriture de position dans le heartbeat (payload
  allégé), position plus fraîche (temps réel vs ~60 s), une seule source de vérité
  pour les coordonnées.

## Les 3 blocages identifiés

1. **`system` et `scene` n'existent pas dans Persistence.** Il n'y a que
   `scenename` (chemin `.tscn`) et `parent_id` (chaîne d'objets). La convention
   hiérarchique `tarsis_1/new-paris` que les zones de mission matchent par préfixe
   (`mission/src/services/zones.service.ts:140-144`) n'y est écrite nulle part.
   Sans elle, les zones `system`/`poi` ne peuvent pas matcher.
   Pistes : (a) heartbeat garde `system`+`scene`, seule la `position` vient de
   Persistence *(hybride, recommandé, aucun changement de contrat serveur de jeu)* ;
   (b) migration complète — le jeu écrit `system`/`scene` dans l'objet joueur de
   Persistence ; (c) déduction par remontée de `parent_id` *(fragile, N+1)*.
2. **Persistence n'a ni auth, ni endpoint batch.** Son API REST est totalement
   ouverte (`persistence/src/rest/mod.rs:12-21`, aucune auth) et `GET /items` fait
   un full table scan filtré en mémoire (`persistence/src/rest/handlers.rs:132`) —
   inutilisable pour lire 20 positions d'amis ou 100 membres de corpo.
   Il faudrait : auth service Keycloak (`client_credentials`, rôles capacité,
   comme partout ailleurs) + `POST /api/internal/players/positions {ids}` en
   point-lookup Scylla.
3. **Le cache le plus frais n'est pas exposé en REST.** `GET /items/:uuid` lit
   Scylla directement (`persistence/src/rest/handlers.rs:178`) sans passer par le
   DualCache où vivent les positions à jour (jusqu'à `CACHE_FLUSH_INTERVAL_SECS`,
   défaut 60 s, avant flush). Une lecture REST serait donc *moins* fraîche que
   l'heartbeat actuel — il faudrait un chemin lecture cache-first (le chemin WS
   `update_object` fait déjà `cache.get` d'abord, `handlers.rs:103`).

## Conséquences si c'est fait un jour

- **Mission** : aucun changement de contrat s'il continue de ne parler qu'à Social
  (`GET /api/internal/players/:id/presence` renvoie toujours `location` fusionnée).
- **Dégradation** : panne Persistence → `location: null` → fail-closed sur les zones
  (seules les missions globales), comportement déjà en place pour une panne Social.
- **Joueur hors-ligne** : sa dernière position reste dans Persistence → règle
  obligatoire « location seulement si `status != offline` » (déjà la sémantique de
  `staleToOffline` dans `src/services/presence.service.ts`).
- **Colonne `player_presence.location`** : à l'arrêt de l'écriture heartbeat, garder
  la colonne en rétrocompatibilité le temps d'une transition, puis migration de dépôt.
- **Lot de travail Persistence (Rust)** : auth + endpoint batch + lecture
  cache-first + doc OpenAPI. C'est le gros du chantier, d'où le statut « rien n'est fait ».

## Décisions à trancher avant toute implémentation

1. Origine de `system`/`scene` : hybride (heartbeat garde system+scene) / migration
   complète côté jeu / déduction par `parent_id`.
2. Point de lecture : Social fait le join *(recommandé)* / chaque service lit Persistence.
3. Modèle d'auth Persistence : Keycloak service accounts *(recommandé)* /
   confiance réseau interne.
