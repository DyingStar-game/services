package uidresolver

import (
	"context"
	"dyingstar/services/persistance/infra/cache"
	"dyingstar/services/persistance/infra/database"
	"encoding/json"
)

// Resolve retourne l'UID pour un UUID donné (cache + DB)
func Resolve(uuid string) (string, error) {
	// Cache d'abord
	if uid, found := cache.GetInstance().Get(uuid); found {
		return uid, nil
	}

	// Puis DB
	client := database.GetClient()
	txn := client.NewReadOnlyTxn()
	ctx := context.Background()
	defer txn.Discard(ctx)

	variables := map[string]string{"$uuid": uuid}
	q := `
	query EntityByUuid($uuid: string) {
		entity(func: eq(uuid, $uuid)) {
			uid
		}
	}
	`

	resp, err := txn.QueryWithVars(ctx, q, variables)
	if err != nil {
		return "", err
	}

	var result struct {
		Entity []struct {
			Uid string `json:"uid"`
		} `json:"entity"`
	}

	err = json.Unmarshal(resp.Json, &result)
	if err != nil {
		return "", err
	}

	if len(result.Entity) == 0 {
		return "", nil // Pas d'erreur, juste pas trouvé
	}

	uid := result.Entity[0].Uid
	cache.GetInstance().Set(uuid, uid)

	return uid, nil
}
