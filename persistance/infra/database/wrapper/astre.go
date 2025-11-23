package wrapper

import (
	"dyingstar/services/persistance/domain/models"
	"dyingstar/services/persistance/infra/database/uidresolver"
)

type PlanetWraper struct {
	CommonWraper
	models.Planet
}

func (pw *PlanetWraper) Wrap(obj IDgraph) {
	if planet, ok := obj.(*models.Planet); ok {
		if uid, err := uidresolver.Resolve(planet.GetUuid()); err == nil && uid != "" {
			pw.SetUid(uid)
		}
		pw.Name = planet.Name
		pw.Uuid = planet.Uuid

		// position
		pw.X = planet.X
		pw.Y = planet.Y
		pw.Z = planet.Z
		pw.Rx = planet.Rx
		pw.Ry = planet.Ry
		pw.Rz = planet.Rz
		// dgraph field
		pw.DType = []string{"Planet", "Entity", "Position"}
		pw.TypeGo = obj.TypeName()
	} else {
		panic("Is not a Planet")
	}
}

func (pw *PlanetWraper) UnWrap() IDgraph {
	planet := &models.Planet{}
	planet.Name = pw.Name
	planet.Uuid = pw.Uuid
	// position
	planet.X = pw.X
	planet.Y = pw.Y
	planet.Z = pw.Z
	planet.Rx = pw.Rx
	planet.Ry = pw.Ry
	planet.Rz = pw.Rz
	return planet
}

func newPlanetWrapper() IWrapper {
	return &PlanetWraper{}
}

type SystemWraper struct {
	CommonWraper
	models.System
}

func (pw *SystemWraper) Wrap(obj IDgraph) {
	if system, ok := obj.(*models.System); ok {
		if uid, err := uidresolver.Resolve(system.GetUuid()); err == nil && uid != "" {
			pw.SetUid(uid)
		}
		pw.Name = system.Name
		pw.Uuid = system.Uuid
		pw.DType = []string{"Entity", "Position"}
		pw.TypeGo = obj.TypeName()
	} else {
		panic("Is not a system")
	}
}

func (pw *SystemWraper) UnWrap() IDgraph {
	system := &models.System{}
	system.Name = pw.Name
	system.Uuid = pw.Uuid

	return system
}

func newSystemWrapper() IWrapper {
	return &SystemWraper{}
}
