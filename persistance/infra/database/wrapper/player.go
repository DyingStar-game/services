package wrapper

import (
	"dyingstar/services/persistance/domain/models"
	"dyingstar/services/persistance/infra/database/uidresolver"
)

type PlayerWraper struct {
	CommonWraper
	models.Player
}

func (pw *PlayerWraper) Wrap(obj IDgraph) {
	if player, ok := obj.(*models.Player); ok {
		if uid, err := uidresolver.Resolve(player.GetUuid()); err == nil && uid != "" {
			pw.SetUid(uid)
		}
		pw.Username = player.Username
		pw.Uuid = player.Uuid
		pw.DType = []string{"Player", "Entity", "Position"}
		pw.TypeGo = obj.TypeName()
	} else {
		panic("Is not a player")
	}
}

func (pw *PlayerWraper) UnWrap() IDgraph {
	player := &models.Player{}
	player.Username = pw.Username
	player.Uuid = pw.Uuid
	return player
}

func newPlayerWrapper() IWrapper {
	return &PlayerWraper{}
}
