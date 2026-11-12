// Fixture generation only: independent HPKE sender from Go's standard library.
package main

import (
    "crypto/ecdh"
    "crypto/hpke"
    "encoding/hex"
    "encoding/json"
    "os"
)

func main() {
    data, err := os.ReadFile(os.Args[1]); if err != nil { panic(err) }
    var vector struct { Secret string `json:"skRm"` }
    if err = json.Unmarshal(data, &vector); err != nil { panic(err) }
    scalar, err := hex.DecodeString(vector.Secret); if err != nil { panic(err) }
    private, err := ecdh.P256().NewPrivateKey(scalar); if err != nil { panic(err) }
    public, err := hpke.NewDHKEMPublicKey(private.PublicKey()); if err != nil { panic(err) }
    encapsulated, sender, err := hpke.NewSender(public,hpke.HKDFSHA256(),hpke.AES256GCM(),nil)
    if err != nil { panic(err) }
    key := make([]byte,32); for index := range key { key[index]=byte(index) }
    wrapped, err := sender.Seal(nil,key); if err != nil { panic(err) }
    if err = json.NewEncoder(os.Stdout).Encode(map[string][]byte{"enc-request":encapsulated,"wrapped-key":wrapped}); err != nil { panic(err) }
}
