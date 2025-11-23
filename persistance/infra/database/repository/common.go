package datarepository

import (
	"context"
	"dyingstar/services/persistance/infra/cache"
	"dyingstar/services/persistance/infra/database"
	"dyingstar/services/persistance/infra/database/uidresolver"
	"dyingstar/services/persistance/infra/database/wrapper"
	"encoding/json"
	"fmt"
	"log"

	"github.com/dgraph-io/dgo/v240"
	"github.com/dgraph-io/dgo/v240/protos/api"
)

type Repository[T wrapper.IDgraph] struct {
	client *dgo.Dgraph
}

func NewCommonRepository[T wrapper.IDgraph]() *Repository[T] {
	return &Repository[T]{
		client: database.GetClient(),
	}
}

func (r Repository[T]) Save(obj T) {
	fmt.Print("save on data\n")
	dstruct := wrapper.MapToWrapper(obj)
	dstruct.Wrap(obj)
	if dstruct.GetUid() == "" {
		dstruct.SetUid("_:temp0")
		dg, err := json.Marshal(dstruct)
		if err != nil {
			log.Fatal(err)
		}
		txn := r.client.NewTxn()
		ctx := context.Background()
		defer txn.Discard(ctx)
		resp, err := txn.Mutate(ctx, &api.Mutation{SetJson: dg, CommitNow: true})
		if err != nil {
			log.Fatal(err)
		}
		dstruct.SetUid(resp.GetUids()["temp0"])
	} else {
		r.bashSave(obj)
	}
}

func (r Repository[T]) LoadByUId(uid string) T {
	print("load by Id on data\n")

	// 1. Charger les données brutes
	rawData := r.fetchEntityData(uid)

	// 2. Déterminer le type et créer le wrapper
	wrapperInstance := r.createWrapperFromData(rawData)

	// 3. Unmarshaler dans le wrapper
	r.unmarshalIntoWrapper(rawData, wrapperInstance)

	// 4. Mettre en cache
	entity := wrapperInstance.UnWrap()
	cache.GetInstance().Set(entity.GetUuid(), uid)

	// 5. Retourner l'entité unwrapped
	return entity.(T)
}

func (r Repository[T]) fetchEntityData(uid string) []byte {
	txn := r.client.NewReadOnlyTxn()
	ctx := context.Background()
	defer txn.Discard(ctx)

	variables := map[string]string{"$id1": uid}
	q := `
	query Entity($id1: string) {
		entity(func: uid($id1)) {
			uid
			uuid
			name
			type_go
			x
			y
			z
			dgraph.type
		}
	}
	`

	resp, err := txn.QueryWithVars(ctx, q, variables)
	if err != nil {
		log.Fatal(err)
	}

	return resp.Json
}

func (r Repository[T]) createWrapperFromData(jsonData []byte) wrapper.IWrapper {
	var temp struct {
		Entity []struct {
			TypeGo string `json:"type_go"`
		} `json:"entity"`
	}

	err := json.Unmarshal(jsonData, &temp)
	if err != nil || len(temp.Entity) == 0 {
		log.Fatal("unable to determine entity type")
	}

	typeName := temp.Entity[0].TypeGo
	factory, ok := wrapper.GetWrapperFactory(typeName)
	if !ok {
		log.Fatal("wrapper not registered for type: " + typeName)
	}

	return factory()
}

func (r Repository[T]) unmarshalIntoWrapper(jsonData []byte, w wrapper.IWrapper) {
	var result struct {
		Entity []json.RawMessage `json:"entity"`
	}

	err := json.Unmarshal(jsonData, &result)
	if err != nil || len(result.Entity) == 0 {
		log.Fatal("unable to unmarshal entity data")
	}

	err = json.Unmarshal(result.Entity[0], w)
	if err != nil {
		log.Fatal("unable to unmarshal into wrapper: ", err)
	}
}

func (r Repository[T]) DeleteByUId(uid string) {
}

func (r Repository[T]) LoadByUUID(uuid string) (T, error) {
	var zero T

	// Chercher l'UID
	uid, err := uidresolver.Resolve(uuid)
	if err != nil {
		return zero, err
	}

	// Charger par UID
	return r.LoadByUId(uid), nil
}

func (r Repository[T]) bashSave(obj T) {
	fmt.Print("bash save on data")
}
