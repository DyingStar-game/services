# services

Services in relation with Horizon server used to manage the different gameplays, the props...

## OpenAPI agrégé

Le fichier `openapi.yaml` à la racine est un **document agrégé** de tous les endpoints des services
`mission`, `social`, `economie`, `inventory` et `market` (public, joueur, `/api/internal/*` et
`/api/admin/*`). Chaque service expose aussi sa propre spec sur `GET /openapi.yaml`.

Pour le régénérer après une modification d'une spec de service :

```bash
python3 scripts/merge-openapi.py   # nécessite PyYAML, écrit ./openapi.yaml
```

Le script préfixe les composants par service (`social_PlayerProfile`, `economie_Account`, …),
fusionne les chemins servis par plusieurs services et déduit les endpoints publics partagés.
