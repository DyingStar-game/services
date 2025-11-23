package models

type MiningRock struct {
	Entity
	Weight float32 `json:"weight"`
	Blocs  string
}

func (mr MiningRock) TypeName() string { return "MiningRock" }
