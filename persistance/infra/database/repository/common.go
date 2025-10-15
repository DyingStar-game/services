package datarepository

import (
	"context"
	"dyingstar/services/persistance/infra/database"
	datamodels "dyingstar/services/persistance/infra/database/models"
	"encoding/json"
	"fmt"
	"log"

	"github.com/dgraph-io/dgo/v240"
	"github.com/dgraph-io/dgo/v240/protos/api"
)

type Repository[T datamodels.DgraphI] struct {
	Client *dgo.Dgraph
}

func NewCommonRepository[T datamodels.DgraphI]() *Repository[T] {
	return &Repository[T]{
		Client: database.GetClient(),
	}
}

func (r Repository[T]) Save(obj T) {
	fmt.Print("save on data\n")

	if obj.GetUid() == "" {
		obj.SetUid("_:temp0")
		dstruct := datamodels.MapToDgraphStruct(obj)
		dg, err := json.Marshal(dstruct)
		if err != nil {
			log.Fatal(err)
		}
		txn := r.Client.NewTxn()
		ctx := context.Background()
		defer txn.Discard(ctx)
		resp, err := txn.Mutate(ctx, &api.Mutation{SetJson: dg, CommitNow: true})
		if err != nil {
			log.Fatal(err)
		}
		obj.SetUid(resp.GetUids()["temp0"])
	} else {
		r.bashSave(obj)
	}

}

func (r Repository[T]) LoadByUId(uid string) T {
	print("load by Id on data\n")

	txn := r.Client.NewReadOnlyTxn()
	ctx := context.Background()
	defer txn.Discard(ctx)
	variables := map[string]string{"$id1": uid}
	q := `
	query Entity($id1: string) {
		entity(func: uid($id1)) {
			uid
			uuid
			name
			children: ~parent {
				uid
				uuid
				dgraph.type
				parent {
					uid
				}
				username
			}
			dgraph.type
		}
	}
	`
	resp, err := txn.QueryWithVars(ctx, q, variables)
	if err != nil {
		log.Fatal(err)
	}

	var result struct {
		Entity []T `json:"Entity"`
	}

	err = json.Unmarshal(resp.Json, &result)
	if err != nil {
		log.Fatal(err)
	}

	return result.Entity[0]
}

func (r Repository[T]) DeleteByUId(uid string) {
}

/*
func (r Repository[T]) LoadByUUID(uuid string) T {
	print("load uuid on data")


}



*/

func (r Repository[T]) bashSave(obj T) {
	fmt.Print("bash save on data")
	fmt.Println(obj.GetUid())
}
