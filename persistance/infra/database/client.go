package database

import (
	"context"
	"dyingstar/services/persistance/app/config"
	"log"
	"os"
	"strconv"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/encoding/gzip"

	"github.com/dgraph-io/dgo/v240"
	"github.com/dgraph-io/dgo/v240/protos/api"
)

func newClient() *dgo.Dgraph {
	cfg := config.GetConfig()
	dialOpts := append([]dgo.ClientOption{},
		dgo.WithGrpcOption(grpc.WithTransportCredentials(insecure.NewCredentials())),
		dgo.WithGrpcOption(grpc.WithDefaultCallOptions(grpc.UseCompressor(gzip.Name))),
	)
	dgoAddr := cfg.Database.Host + ":" + strconv.Itoa(cfg.Database.Port)
	d, err := dgo.NewClient(dgoAddr, dialOpts...)

	if err != nil {
		log.Fatal(err)
	}

	return d
}

func setup(c *dgo.Dgraph) {
	data, err := os.ReadFile("database.schema")
	if err != nil {
		panic(err)
	}
	content := string(data)
	err = c.Alter(context.Background(), &api.Operation{
		Schema: content,
	})
	if err != nil {
		log.Fatal(err)
	}
}

func reset(c *dgo.Dgraph) {
	err := c.Alter(context.Background(), &api.Operation{DropOp: api.Operation_ALL})
	if err != nil {
		log.Fatal(err)
	}
}

func wipe(c *dgo.Dgraph) {
	err := c.Alter(context.Background(), &api.Operation{DropOp: api.Operation_DATA})
	if err != nil {
		log.Fatal(err)
	}
}
