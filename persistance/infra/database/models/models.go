package datamodels

import "dyingstar/services/persistance/domain/models"

func init() {
	RegisterType("Player",
		func() models.IEntity {
			p := models.NewPlayer()
			return &p
		},
		[]string{"Player", "Entity", "Position"},
	)

	RegisterTypeWithHook("Planet",
		func() models.IEntity {
			p := models.NewPlanet()
			return &p
		},
		PolymorphicSliceHook("children"),
		[]string{"Planet", "Entity", "Position"},
	)
}
