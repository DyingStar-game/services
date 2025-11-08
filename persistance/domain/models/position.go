package models

type Position struct {
	X  float64 `json:"x"`
	Y  float64 `json:"y"`
	Z  float64 `json:"z"`
	Rx float64 `json:"rx"`
	Ry float64 `json:"ry"`
	Rz float64 `json:"rz"`
}

func (p *Position) GetPositionVector64() [3]float64 {
	return [3]float64{p.X, p.Y, p.Z}
}

func (p *Position) GetPositionVector32() [3]float32 {
	return [3]float32{float32(p.X), float32(p.Y), float32(p.Z)}
}
