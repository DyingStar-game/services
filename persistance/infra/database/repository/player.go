package datarepository

import (
	"dyingstar/services/persistance/domain/models"
)

type PlayerRepository struct {
	*Repository[*models.Player]
}

func NewPlayerRepository() *PlayerRepository {
	return &PlayerRepository{
		Repository: NewCommonRepository[*models.Player](),
	}
}
