package propsmapper

import (
	"dyingstar/services/persistance/domain/models"
	datarepository "dyingstar/services/persistance/infra/database/repository"
)

var system models.System = models.System{
	Entity: models.Entity{
		Uuid: "3388a817-f3ef-421d-sdsd-4325e105628e",
	},
	Name: "Tarsis",
}

type PlanetMapper struct {
	planet models.Planet
}

func (pm *PlanetMapper) FromMap(uuid string, data map[string]interface{}) {
	dataPosition := data["position"].(map[string]interface{})
	pm.planet = models.NewPlanet(&system, uuid, data["name"].(string), models.Position{
		X: dataPosition["x"].(float64),
		Y: dataPosition["y"].(float64),
		Z: dataPosition["z"].(float64),
	})
}

func (pm *PlanetMapper) ToMap() map[string]interface{} {
	data := map[string]interface{}{}
	return data
}

func (pm *PlanetMapper) Save() {
	repository := datarepository.NewCommonRepository[*models.Planet]()
	repository.Save(&pm.planet)
}

func newPlanetMapper() iPropsMapper { return &PlanetMapper{} }
