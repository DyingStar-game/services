package models

type IEntity interface {
	GetUuid() string
	GetPosition() Position
}

type Entity struct {
	Uuid    string `json:"uuid"`
	ObjType string `json:"type_obj"`
	Position
	Parent IEntity `json:"parent,omitempty"`
}

func (e *Entity) GetUuid() string       { return e.Uuid }
func (e *Entity) GetPosition() Position { return e.Position }

type EntityChild struct {
	Player *Player `json:"player,omitempty"`
}
