package wrapper

func init() {
	RegisterWrapperType("Player", NewPlayerWrapper)
	RegisterWrapperType("Planet", NewPlanetWrapper)
	RegisterWrapperType("System", NewSystemWrapper)
}
