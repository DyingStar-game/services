package models

import "github.com/google/uuid"

type Planet struct {
	Entity
	Name     string    `json:"name,omitempty"`
	Children []IEntity `json:"children,omitempty"`
}

func (p *Planet) GetType() string { return "Planet" }

func NewPlanet() Planet {
	planet := Planet{
		Entity: Entity{
			Position: Position{
				X:  0.0,
				Y:  0.0,
				Z:  0.0,
				Rx: 0.0,
				Ry: 0.0,
				Rz: 0.0,
			},
			Uuid: uuid.New(),
		},
		Name: "testPlanet",
	}
	return planet
}
