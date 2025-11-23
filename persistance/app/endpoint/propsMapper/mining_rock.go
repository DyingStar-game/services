package propsmapper

import (
	"dyingstar/services/persistance/domain/models"
	datarepository "dyingstar/services/persistance/infra/database/repository"
)

type MiningRockMapper struct {
	miningRock models.MiningRock
}

func (mrm *MiningRockMapper) FromMap(uuid string, data map[string]interface{}) {
	dataPosition, asPosition := data["position"].(map[string]interface{})
	mrm.miningRock = models.MiningRock{}

	mrm.miningRock.Uuid = uuid
	if asPosition {
		mrm.miningRock.Position = models.Position{
			X: dataPosition["x"].(float64),
			Y: dataPosition["y"].(float64),
			Z: dataPosition["z"].(float64),
		}
	}
}

func (mrm *MiningRockMapper) ToMap() map[string]interface{} {
	data := map[string]interface{}{}
	return data
}

func (mrm *MiningRockMapper) Save() {
	repository := datarepository.NewCommonRepository[*models.MiningRock]()
	repository.Save(&mrm.miningRock)
}

func newMiningRockMapper() iPropsMapper { return &MiningRockMapper{} }
