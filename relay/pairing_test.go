package main

import (
 "crypto/ed25519"
 "crypto/rand"
 "encoding/base64"
 "encoding/json"
 "net/http"
 "net/http/httptest"
 "path/filepath"
 "strconv"
 "strings"
 "testing"
 "time"
)

const testPairToken = "test-only-relay-token-0123456789abcdef"

func apiRequest(t *testing.T, handler http.Handler, method, path string, body any, auth bool) *httptest.ResponseRecorder {
 t.Helper()
 var payload []byte
 if body != nil { var err error; payload,err=json.Marshal(body);if err!=nil{t.Fatal(err)} }
 req:=httptest.NewRequest(method,path,strings.NewReader(string(payload)))
 if auth { req.Header.Set("Authorization","Bearer "+testPairToken) }
 req.Header.Set("X-Forwarded-Proto","https")
 response:=httptest.NewRecorder();handler.ServeHTTP(response,req);return response
}

func newRegisteredPairingService(t *testing.T)(*pairingService,ed25519.PrivateKey,string,string){
 t.Helper()
 pub,priv,err:=ed25519.GenerateKey(rand.Reader);if err!=nil{t.Fatal(err)}
 path:=filepath.Join(t.TempDir(),"devices.json")
 service,err:=newPairingService(path,testPairToken);if err!=nil{t.Fatal(err)}
 id:=strings.Repeat("a",32)
 response:=apiRequest(t,service.handler(),http.MethodPost,"/api/v1/devices/register",map[string]string{
  "device_id":id,"name":"Test Windows","identity_public_key":base64.RawURLEncoding.EncodeToString(pub),
 },true)
 if response.Code!=http.StatusCreated { t.Fatalf("register status=%d body=%s",response.Code,response.Body.String()) }
 var result struct{DeviceNumber string `json:"device_number"`}
 if err=json.Unmarshal(response.Body.Bytes(),&result);err!=nil{t.Fatal(err)}
 if !deviceNumberPattern.MatchString(result.DeviceNumber){t.Fatalf("invalid assigned device number %q",result.DeviceNumber)}
 return service,priv,id,result.DeviceNumber
}

func TestDeviceRegistrationAssignsStableNineDigitNumber(t *testing.T){
 service,_,id,number:=newRegisteredPairingService(t)
 response:=apiRequest(t,service.handler(),http.MethodPost,"/api/v1/devices/register",map[string]string{
  "device_id":id,"name":"Renamed PC","identity_public_key":service.devices[id].IdentityPublicKey,
 },true)
 if response.Code!=http.StatusOK{t.Fatalf("idempotent registration status=%d",response.Code)}
 var result struct{DeviceNumber string `json:"device_number"`}
 if err:=json.Unmarshal(response.Body.Bytes(),&result);err!=nil{t.Fatal(err)}
 if result.DeviceNumber!=number{t.Fatalf("device number changed: %s != %s",result.DeviceNumber,number)}
 reopened,err:=newPairingService(service.path,testPairToken);if err!=nil{t.Fatal(err)}
 if reopened.numbers[number]!=id{t.Fatal("device registry did not persist across restart")}
}

func TestPairingWindowRequiresDeviceIdentitySignature(t *testing.T){
 service,priv,id,number:=newRegisteredPairingService(t)
 pairingPub:=make([]byte,32);if _,err:=rand.Read(pairingPub);err!=nil{t.Fatal(err)}
 encoded:=base64.RawURLEncoding.EncodeToString(pairingPub)
 expiry:=time.Now().Add(90*time.Second).Unix()
 message:=pairingWindowMessage(number,strconv.FormatInt(expiry,10),encoded)
 signature:=ed25519.Sign(priv,message)
 response:=apiRequest(t,service.handler(),http.MethodPost,"/api/v1/devices/pairing-window",map[string]any{
  "device_id":id,"pairing_public_key":encoded,"signature":base64.RawURLEncoding.EncodeToString(signature),"expires_at":expiry,
 },true)
 if response.Code!=http.StatusOK{t.Fatalf("valid signed pairing window rejected: %d %s",response.Code,response.Body.String())}
 bad:=apiRequest(t,service.handler(),http.MethodPost,"/api/v1/devices/pairing-window",map[string]any{
  "device_id":id,"pairing_public_key":encoded,"signature":base64.RawURLEncoding.EncodeToString(make([]byte,ed25519.SignatureSize)),"expires_at":expiry,
 },true)
 if bad.Code!=http.StatusUnauthorized{t.Fatalf("forged pairing window status=%d, want 401",bad.Code)}
}

func TestPairingRequestRequiresHostApprovalAndConsumesCodeWindow(t *testing.T){
 service,priv,id,number:=newRegisteredPairingService(t)
 pairingPub:=make([]byte,32);_,_=rand.Read(pairingPub);encoded:=base64.RawURLEncoding.EncodeToString(pairingPub)
 expiry:=time.Now().Add(90*time.Second).Unix()
 signature:=ed25519.Sign(priv,pairingWindowMessage(number,strconv.FormatInt(expiry,10),encoded))
 window:=apiRequest(t,service.handler(),http.MethodPost,"/api/v1/devices/pairing-window",map[string]any{
  "device_id":id,"pairing_public_key":encoded,"signature":base64.RawURLEncoding.EncodeToString(signature),"expires_at":expiry,
 },true)
 if window.Code!=200{t.Fatalf("publish window failed: %s",window.Body.String())}
 viewerPub:=make([]byte,32);nonce:=make([]byte,16);proof:=make([]byte,32)
 _,_=rand.Read(viewerPub);_,_=rand.Read(nonce);_,_=rand.Read(proof)
 create:=apiRequest(t,service.handler(),http.MethodPost,"/api/v1/pairing/requests",map[string]string{
  "device_number":number,"viewer_public_key":base64.RawURLEncoding.EncodeToString(viewerPub),
  "nonce":base64.RawURLEncoding.EncodeToString(nonce),"proof":base64.RawURLEncoding.EncodeToString(proof),
 },true)
 if create.Code!=http.StatusCreated{t.Fatalf("create request failed: %d %s",create.Code,create.Body.String())}
 var created struct{RequestID string `json:"request_id"`}
 if err:=json.Unmarshal(create.Body.Bytes(),&created);err!=nil{t.Fatal(err)}
 if !pairRequestPattern.MatchString(created.RequestID){t.Fatalf("invalid request id %q",created.RequestID)}
 list:=apiRequest(t,service.handler(),http.MethodGet,"/api/v1/pairing/requests?device_id="+id,nil,true)
 if list.Code!=200||!strings.Contains(list.Body.String(),created.RequestID){t.Fatalf("host did not receive pending request: %d %s",list.Code,list.Body.String())}
 wrong:=apiRequest(t,service.handler(),http.MethodPost,"/api/v1/pairing/requests/"+created.RequestID+"/decision",map[string]any{"device_id":strings.Repeat("b",32),"approved":true},true)
 if wrong.Code!=http.StatusForbidden{t.Fatalf("other device decision status=%d",wrong.Code)}
 decisionSig:=ed25519.Sign(priv,pairingDecisionMessage(created.RequestID,number,base64.RawURLEncoding.EncodeToString(viewerPub),base64.RawURLEncoding.EncodeToString(nonce),true))
 approved:=apiRequest(t,service.handler(),http.MethodPost,"/api/v1/pairing/requests/"+created.RequestID+"/decision",map[string]any{
  "device_id":id,"approved":true,"signature":base64.RawURLEncoding.EncodeToString(decisionSig),
 },true)
 if approved.Code!=200{t.Fatalf("host approval failed: %s",approved.Body.String())}
 status:=apiRequest(t,service.handler(),http.MethodGet,"/api/v1/pairing/requests/"+created.RequestID,nil,true)
 if status.Code!=200||!strings.Contains(status.Body.String(),`"status":"approved"`){t.Fatalf("viewer status incorrect: %d %s",status.Code,status.Body.String())}
 second:=apiRequest(t,service.handler(),http.MethodPost,"/api/v1/pairing/requests",map[string]string{
  "device_number":number,"viewer_public_key":base64.RawURLEncoding.EncodeToString(viewerPub),
  "nonce":base64.RawURLEncoding.EncodeToString(nonce),"proof":base64.RawURLEncoding.EncodeToString(proof),
 },true)
 if second.Code!=http.StatusNotFound{t.Fatalf("pairing code window was reusable: status=%d",second.Code)}
}

func TestPairingRequestsAreRateLimited(t *testing.T){
 service,priv,id,number:=newRegisteredPairingService(t)
 pairingPub:=make([]byte,32);_,_=rand.Read(pairingPub);encoded:=base64.RawURLEncoding.EncodeToString(pairingPub)
 expiry:=time.Now().Add(90*time.Second).Unix()
 sig:=ed25519.Sign(priv,pairingWindowMessage(number,strconv.FormatInt(expiry,10),encoded))
 window:=apiRequest(t,service.handler(),http.MethodPost,"/api/v1/devices/pairing-window",map[string]any{
  "device_id":id,"pairing_public_key":encoded,"signature":base64.RawURLEncoding.EncodeToString(sig),"expires_at":expiry,
 },true)
 if window.Code!=200{t.Fatal(window.Body.String())}
 var last int
 for i:=0;i<6;i++{
  viewerPub:=make([]byte,32);nonce:=make([]byte,16);proof:=make([]byte,32);_,_=rand.Read(viewerPub);_,_=rand.Read(nonce);_,_=rand.Read(proof)
  response:=apiRequest(t,service.handler(),http.MethodPost,"/api/v1/pairing/requests",map[string]string{
   "device_number":number,"viewer_public_key":base64.RawURLEncoding.EncodeToString(viewerPub),
   "nonce":base64.RawURLEncoding.EncodeToString(nonce),"proof":base64.RawURLEncoding.EncodeToString(proof),
  },true)
  last=response.Code
 }
 if last!=http.StatusTooManyRequests{t.Fatalf("sixth attempt status=%d, want 429",last)}
}

func TestPairingAPIRequiresBearerTokenAndTLS(t *testing.T){
 service,_,_,_:=newRegisteredPairingService(t)
 unauthorized:=apiRequest(t,service.handler(),http.MethodGet,"/api/v1/devices/123456789",nil,false)
 if unauthorized.Code!=http.StatusUnauthorized{t.Fatalf("unauthorized status=%d",unauthorized.Code)}
 req:=httptest.NewRequest(http.MethodGet,"/api/v1/devices/123456789",nil)
 req.Header.Set("Authorization","Bearer "+testPairToken);req.RemoteAddr="203.0.113.10:1234"
 response:=httptest.NewRecorder();service.handler().ServeHTTP(response,req)
 if response.Code!=http.StatusUpgradeRequired{t.Fatalf("plain HTTP status=%d, want 426",response.Code)}
}
