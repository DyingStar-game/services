package wrapper

import (
	"dyingstar/services/persistance/domain/models"
	"dyingstar/services/persistance/infra/database/uidresolver"
)

type BoxWrapper struct {
	CommonWraper
	models.Box
}

func (bw *BoxWrapper) Wrap(obj IDgraph) {
	if box, ok := obj.(*models.Box); ok {
		if uid, err := uidresolver.Resolve(box.GetUuid()); err == nil && uid != "" {
			bw.SetUid(uid)
		}

		bw.Uuid = box.Uuid
		// position
		bw.X = box.X
		bw.Y = box.Y
		bw.Z = box.Z
		bw.Rx = box.Rx
		bw.Ry = box.Ry
		bw.Rz = box.Rz
		// dgraph field
		bw.DType = []string{"Box", "Entity", "Position"}
		bw.TypeGo = obj.TypeName()
		bw.ObjType = box.ObjType
	} else {
		panic("Is not a box")
	}
}

func (bw *BoxWrapper) UnWrap() IDgraph {
	box := &models.Box{}
	box.Uuid = bw.Uuid
	// position
	box.X = bw.X
	box.Y = bw.Y
	box.Z = bw.Z
	box.Rx = bw.Rx
	box.Ry = bw.Ry
	box.Rz = bw.Rz

	box.ObjType = bw.ObjType
	return box
}

func newBoxWrapper() IWrapper {
	return &BoxWrapper{}
}
