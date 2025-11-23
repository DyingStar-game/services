package wrapper

func init() {
	registerWrapperType("Player", newPlayerWrapper)
	registerWrapperType("Planet", newPlanetWrapper)
	registerWrapperType("System", newSystemWrapper)
	registerWrapperType("Box", newBoxWrapper)
	registerWrapperType("MiningRock", newMiningRockWrapper)
}
