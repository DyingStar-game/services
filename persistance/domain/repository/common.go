package repository

type Repository[T any] interface {
	Save(obj *T)
	LoadByUUID(uuid string) T
	LoadByUId(uid string) T
	DeleteByUId(uid string)
}
