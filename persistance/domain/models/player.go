package models

import "github.com/google/uuid"

type Player struct {
	Entity
	Username string `json:"username,omitempty"`
}

func (p *Player) GetType() string { return "Player" }

func NewPlayer() Player {
	player := Player{
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
		Username: "testUsername",
	}
	return player
}
