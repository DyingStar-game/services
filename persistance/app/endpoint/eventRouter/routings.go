package eventRouter

func init() {
	registerEventRouter("genericprops", newPropsRouter)
}
