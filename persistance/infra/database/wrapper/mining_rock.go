package wrapper

import (
	"dyingstar/services/persistance/domain/models"
	"dyingstar/services/persistance/infra/database/uidresolver"
)

type MiningRockWrapper struct {
	CommonWraper
	models.MiningRock
}

func (mrw *MiningRockWrapper) Wrap(obj IDgraph) {
	if miningRock, ok := obj.(*models.MiningRock); ok {
		if uid, err := uidresolver.Resolve(miningRock.GetUuid()); err == nil && uid != "" {
			mrw.SetUid(uid)
		}
		mrw.Uuid = miningRock.Uuid
		// position
		mrw.X = miningRock.X
		mrw.Y = miningRock.Y
		mrw.Z = miningRock.Z
		mrw.Rx = miningRock.Rx
		mrw.Ry = miningRock.Ry
		mrw.Rz = miningRock.Rz
		// dgraph field
		mrw.DType = []string{"MiningRock", "Entity", "Position"}
		mrw.TypeGo = obj.TypeName()
		mrw.ObjType = miningRock.ObjType
	} else {
		panic("Is not a miningRock")
	}
}

func (mrw *MiningRockWrapper) UnWrap() IDgraph {
	miningRock := &models.MiningRock{}
	miningRock.Uuid = mrw.Uuid
	// position
	miningRock.X = mrw.X
	miningRock.Y = mrw.Y
	miningRock.Z = mrw.Z
	miningRock.Rx = mrw.Rx
	miningRock.Ry = mrw.Ry
	miningRock.Rz = mrw.Rz

	miningRock.ObjType = mrw.ObjType
	return miningRock
}

func newMiningRockWrapper() IWrapper {
	return &MiningRockWrapper{}
}
