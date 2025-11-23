package eventRouter

import "fmt"

type eventRouterFactory func() iEventRouter

type iEventRouter interface {
	Routing(eventType string, data map[string]interface{})
}

var EventRouterFactoryRegistry = make(map[string]eventRouterFactory)

func registerEventRouter(name string, factory eventRouterFactory) {
	EventRouterFactoryRegistry[name] = factory
}

func EventRouting(src string, eventType string, data map[string]interface{}) {
	if src == "plugin" {
		factory, ok := EventRouterFactoryRegistry[data["plugin"].(string)]
		if !ok {
			fmt.Println("event routing is not register " + src)
			return
		}
		router := factory()
		router.Routing(eventType, data)
	}

}
