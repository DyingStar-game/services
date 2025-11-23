package propsmapper

import (
	"dyingstar/services/persistance/domain/models"
	datarepository "dyingstar/services/persistance/infra/database/repository"
)

type BoxMapper struct {
	box models.Box
}

func (bm *BoxMapper) FromMap(uuid string, data map[string]interface{}) {
	dataPosition, asPosition := data["position"].(map[string]interface{})
	bm.box = models.Box{}
	bm.box.Uuid = uuid
	if asPosition {
		bm.box.Position = models.Position{
			X: dataPosition["x"].(float64),
			Y: dataPosition["y"].(float64),
			Z: dataPosition["z"].(float64),
		}
	}
}

func (bm *BoxMapper) ToMap() map[string]interface{} {
	data := map[string]interface{}{}
	return data
}

func (bm *BoxMapper) Save() {
	repository := datarepository.NewCommonRepository[*models.Box]()
	repository.Save(&bm.box)
}

func newBoxMapper() iPropsMapper { return &BoxMapper{} }
