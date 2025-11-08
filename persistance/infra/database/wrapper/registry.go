package wrapper

type WrapperFactory func() IWrapper

var wrapperFactoryRegistry = make(map[string]WrapperFactory)

func RegisterWrapperType(typeName string, factory WrapperFactory) {
	wrapperFactoryRegistry[typeName] = factory
}

func MapToWrapper[T IDgraph](obj T) IWrapper {
	typeName := getTypeName(obj)
	factory, ok := wrapperFactoryRegistry[typeName]
	if !ok {
		panic("Type not registered: " + typeName)
	}

	wrapper := factory()
	return wrapper
}
func getTypeName[T IDgraph](obj T) string {
	return obj.TypeName()
}

func GetWrapperFactory(typeName string) (WrapperFactory, bool) {
	factory, ok := wrapperFactoryRegistry[typeName]
	return factory, ok
}
