package models

import "github.com/google/uuid"

type Position struct {
	X  float64 `json:"x"`
	Y  float64 `json:"y"`
	Z  float64 `json:"z"`
	Rx float64 `json:"rx"`
	Ry float64 `json:"ry"`
	Rz float64 `json:"rz"`
}

type IEntity interface {
	GetUid() string
	SetUid(uid string)
	GetType() string
}

type Entity struct {
	Uuid     uuid.UUID `json:"uuid,omitempty"`
	Uid      string    `json:"uid,omitempty"`
	Type_obj string    `json:"type_obj,omitempty"`
	Position
	Parent *Entity `json:"parent,omitempty"`
}

func (e *Entity) GetUid() string    { return e.Uid }
func (e *Entity) SetUid(uid string) { e.Uid = uid }
func (e *Entity) GetType() string   { return "Entity" }
