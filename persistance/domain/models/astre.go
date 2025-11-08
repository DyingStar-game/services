package models

type System struct {
	Entity
	Name    string `json:"name"`
	Planets []Planet
}

func (s *System) AddPlanets(p Planet) {
	s.Planets = append(s.Planets, p)
}

func (p System) TypeName() string { return "System" }

type Planet struct {
	Entity
	Name string `json:"name"`
}

func (p Planet) TypeName() string { return "Planet" }

func NewSystem(uuid string, name string) System {
	system := System{
		Entity: Entity{
			Position: Position{},
			Uuid:     uuid,
		},
		Name: name,
	}
	return system
}

func NewPlanet(system *System, uuid string, name string, position Position) Planet {
	planet := Planet{
		Entity: Entity{
			Position: position,
			Uuid:     uuid,
			Parent:   system,
		},
		Name: name,
	}
	system.AddPlanets(planet)
	return planet
}
