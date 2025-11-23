package eventRouter

import (
	propsmapper "dyingstar/services/persistance/app/endpoint/propsMapper"
	"fmt"
)

type PropsRouter struct{}

func (pr PropsRouter) Routing(eventType string, data map[string]interface{}) {
	switch eventType {
	case "create_object":
		savebyObjectType(data)
	case "update_object":
		savebyObjectType(data)
	default:
		fmt.Println("this eventype is unknow" + eventType)
	}
}

func savebyObjectType(data map[string]interface{}) {
	uuid, uuidExists := data["object_uuid"].(string)
	objType, typeExists := data["object_type"].(string)
	if uuidExists {
		mapper := propsmapper.PropsMapping(objType)
		if mapper == nil {
			fmt.Println("this type is not savable" + data["object_type"].(string))
			return
		}
		mapper.FromMap(uuid, data["object_data"].(map[string]interface{}))
		mapper.Save()
	} else {
		if typeExists {
			fmt.Println("the " + data["object_type"].(string) + "no have uuid")
		} else {
			fmt.Println("Error not info on event")
		}
	}

}

func newPropsRouter() iEventRouter { return &PropsRouter{} }
