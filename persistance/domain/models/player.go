package models

//easyjson:json
type Player struct {
	Entity
	Username         string `json:"username"`
	StreamingContext Entity
}

func NewPlayer(uuid string, username string, position Position) Player {
	player := Player{
		Entity: Entity{
			Position: position,
			Uuid:     uuid,
		},
		Username: username,
	}
	return player
}

func (p Player) TypeName() string { return "Player" }
