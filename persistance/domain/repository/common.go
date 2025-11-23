package repository

type Repository[T any] interface {
	Save(obj *T)
	LoadByUUID(uuid string) T
	DeleteByUUID(uuid string)
}
