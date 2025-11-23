package models

type Box struct {
	Entity
	Opened       bool    `json:"opened"`
	LedState     string  `json:"led_state"`
	Qrcode       string  `json:"qrcode"`
	Symbol       string  `json:"symbol"`
	ParcelNumber int     `json:"parcel_number"`
	Weight       float32 `json:"weight"`
}

func (b Box) TypeName() string { return "Box" }
