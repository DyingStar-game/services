package serialisation

import (
	"github.com/goccy/go-json"
)

// Struct → map via go-json
func StructToMap(v interface{}) (map[string]interface{}, error) {
	b, err := json.Marshal(v)
	if err != nil {
		return nil, err
	}
	var m map[string]interface{}
	err = json.Unmarshal(b, &m)
	return m, err
}

// Compare deux maps et ne garde que les champs modifiés
func DiffMaps(oldMap, newMap map[string]interface{}) map[string]interface{} {
	diff := make(map[string]interface{})
	for k, newVal := range newMap {
		oldVal, exists := oldMap[k]
		if !exists || !deepEqual(oldVal, newVal) {
			diff[k] = newVal
		}
	}
	return diff
}

// Deep compare pour valeurs imbriquées (maps, slices)
func deepEqual(a, b interface{}) bool {
	ab, _ := json.Marshal(a)
	bb, _ := json.Marshal(b)
	return string(ab) == string(bb)
}
