package datamodels

import (
	"dyingstar/services/persistance/domain/models"
	"encoding/json"
)

type DgraphMapper[T DgraphI] func(obj T) DgraphStruct[T]

type DgraphI interface {
	GetUid() string
	SetUid(uid string)
	GetType() string
}

type DgraphStruct[T DgraphI] struct {
	Obj   T        `json:"-"`
	DType []string `json:"dgraph.type,omitempty"`
}

// Crée un DgraphStruct à partir d'un objet
// Utilise les DgraphTypes du registry
func MapToDgraphStruct[T DgraphI](obj T) DgraphStruct[T] {
	entityTypeName := obj.GetType()
	dgraphTypes := GetDgraphTypes(entityTypeName)

	return DgraphStruct[T]{
		Obj:   obj,
		DType: dgraphTypes,
	}
}

// MarshalJSON : sérialise en aplatissant Obj dans le JSON avec dgraph.type et type_go
func (d DgraphStruct[T]) MarshalJSON() ([]byte, error) {
	// Sérialise l'objet
	objBytes, err := json.Marshal(d.Obj)
	if err != nil {
		return nil, err
	}

	// Parse dans une map pour ajouter les champs supplémentaires
	var rawMap map[string]json.RawMessage
	if err := json.Unmarshal(objBytes, &rawMap); err != nil {
		return nil, err
	}

	// Ajoute dgraph.type
	if len(d.DType) > 0 {
		dTypeBytes, _ := json.Marshal(d.DType)
		rawMap["dgraph.type"] = dTypeBytes
	}

	// Ajoute type_go pour la désérialisation polymorphique
	if entity, ok := any(d.Obj).(models.IEntity); ok {
		typeBytes, _ := json.Marshal(entity.GetType())
		rawMap["type_go"] = typeBytes
	}

	return json.Marshal(rawMap)
}

// UnmarshalJSON : désérialise en recréant l'objet avec le bon type polymorphique
func (d *DgraphStruct[T]) UnmarshalJSON(data []byte) error {
	// Lit dgraph.type
	var temp struct {
		DType []string `json:"dgraph.type,omitempty"`
	}

	if err := json.Unmarshal(data, &temp); err != nil {
		return err
	}

	d.DType = temp.DType

	// Utilise le système de registry pour désérialiser polymorphiquement
	entity, err := UnmarshalEntity(data)
	if err != nil {
		return err
	}

	// Cast vers le type T
	d.Obj = entity.(T)
	return nil
}
