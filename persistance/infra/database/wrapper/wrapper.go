package wrapper

type IDgraph interface {
	GetUuid() string
	TypeName() string
}

type CommonWraper struct {
	Uid    string   `json:"uid,omitempty"`
	TypeGo string   `json:"type_go,omitempty"`
	DType  []string `json:"dgraph.type,omitempty"`
}

func (cw *CommonWraper) GetUid() string { return cw.Uid }

func (cw *CommonWraper) SetUid(uid string) { cw.Uid = uid }

type IWrapper interface {
	Wrap(obj IDgraph)
	UnWrap() IDgraph
	GetUid() string
	SetUid(uid string)
}
