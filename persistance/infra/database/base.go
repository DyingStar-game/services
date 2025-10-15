package database

import "github.com/dgraph-io/dgo/v240"

func Init() {
	d := newClient()
	setup(d)
}

func Wipe() {
	d := newClient()
	wipe(d)
}

func ResetData() {
	d := newClient()
	reset(d)
}

func GetClient() *dgo.Dgraph {
	return newClient()
}
