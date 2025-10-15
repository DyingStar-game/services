package datamodels

import (
	"dyingstar/services/persistance/domain/models"
	"encoding/json"
	"fmt"
	"reflect"
)

// Factory pour créer une nouvelle instance d'un type
type EntityFactory func() models.IEntity

// Hook de désérialisation pour gérer les cas spéciaux
type UnmarshalHook func(data []byte, target models.IEntity) error

// Configuration d'un type enregistré
type TypeConfig struct {
	Factory       EntityFactory
	UnmarshalHook UnmarshalHook
	DgraphTypes   []string // Les types Dgraph pour ce type
}

var typeRegistry = make(map[string]*TypeConfig)

// Enregistre un type avec ses DgraphTypes
func RegisterType(typeName string, factory EntityFactory, dgraphTypes []string) {
	typeRegistry[typeName] = &TypeConfig{
		Factory:     factory,
		DgraphTypes: dgraphTypes,
	}
}

// Enregistre un type avec un hook de désérialisation personnalisé
func RegisterTypeWithHook(typeName string, factory EntityFactory, hook UnmarshalHook, dgraphTypes []string) {
	typeRegistry[typeName] = &TypeConfig{
		Factory:       factory,
		UnmarshalHook: hook,
		DgraphTypes:   dgraphTypes,
	}
}

// Récupère les DgraphTypes pour un type donné
func GetDgraphTypes(typeName string) []string {
	if config, ok := typeRegistry[typeName]; ok {
		return config.DgraphTypes
	}
	return []string{"Entity", "Position"} // Défaut
}

// Hook générique pour les types qui ont des slices d'IEntity
func PolymorphicSliceHook(sliceFieldName string) UnmarshalHook {
	return func(data []byte, target models.IEntity) error {
		// Parse le JSON dans une map temporaire
		var rawMap map[string]json.RawMessage
		if err := json.Unmarshal(data, &rawMap); err != nil {
			return err
		}

		// Récupère le slice brut et supprime-le de la map
		childrenRaw, hasChildren := rawMap[sliceFieldName]
		delete(rawMap, sliceFieldName) // Supprime AVANT la désérialisation

		// Désérialise d'abord l'objet sans le slice
		cleanedData, _ := json.Marshal(rawMap)
		if err := json.Unmarshal(cleanedData, target); err != nil {
			return err
		}

		// Si pas de children, on s'arrête là
		if !hasChildren {
			return nil
		}

		// Désérialise le slice polymorphique
		var childrenData []json.RawMessage
		if err := json.Unmarshal(childrenRaw, &childrenData); err != nil {
			return err
		}

		children := make([]models.IEntity, 0, len(childrenData))
		for _, childData := range childrenData {
			child, err := UnmarshalEntity(childData)
			if err != nil {
				return fmt.Errorf("failed to unmarshal child: %w", err)
			}
			children = append(children, child)
		}

		// Utilise la réflexion pour setter le slice
		v := reflect.ValueOf(target).Elem()
		field := v.FieldByName(capitalizeFirst(sliceFieldName))
		if !field.IsValid() {
			return fmt.Errorf("field %s not found", sliceFieldName)
		}

		childrenSlice := reflect.MakeSlice(field.Type(), len(children), len(children))
		for i, child := range children {
			childrenSlice.Index(i).Set(reflect.ValueOf(child))
		}
		field.Set(childrenSlice)

		return nil
	}
}

func capitalizeFirst(s string) string {
	if len(s) == 0 {
		return s
	}
	return string(s[0]-32) + s[1:]
}

// Désérialise n'importe quel entity de manière polymorphique
func UnmarshalEntity(data []byte) (models.IEntity, error) {
	var typeInfo struct {
		TypeGO string `json:"type_go"`
	}

	if err := json.Unmarshal(data, &typeInfo); err != nil {
		return nil, err
	}

	config, ok := typeRegistry[typeInfo.TypeGO]
	if !ok {
		return nil, fmt.Errorf("unknown entity type: %s", typeInfo.TypeGO)
	}

	entity := config.Factory()

	// Utilise le hook si présent, sinon désérialisation standard
	if config.UnmarshalHook != nil {
		if err := config.UnmarshalHook(data, entity); err != nil {
			return nil, err
		}
	} else {
		if err := json.Unmarshal(data, entity); err != nil {
			return nil, err
		}
	}

	return entity, nil
}
