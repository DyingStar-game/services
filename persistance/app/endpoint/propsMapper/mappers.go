package propsmapper

import "fmt"

func init() {
	registerPropsMapper("planet", newPlanetMapper)
	registerPropsMapper("box", newBoxMapper)
	registerPropsMapper("miningrock", newMiningRockMapper)
}

type propsMapperFactory func() iPropsMapper

type iPropsMapper interface {
	FromMap(uuid string, data map[string]interface{})
	ToMap() map[string]interface{}
	Save()
}

var PropsMapperFactoryRegistry = make(map[string]propsMapperFactory)

func registerPropsMapper(name string, factory propsMapperFactory) {
	PropsMapperFactoryRegistry[name] = factory
}

func PropsMapping(mapperName string) iPropsMapper {
	factory, ok := PropsMapperFactoryRegistry[mapperName]
	if !ok {
		fmt.Println("event routing is not register " + mapperName)
		return nil
	}
	return factory()
}
