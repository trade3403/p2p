package main

import (
 "crypto/ed25519"
 "crypto/rand"
 "crypto/subtle"
 "encoding/base64"
 "encoding/hex"
 "encoding/json"
 "errors"
 "fmt"
 "io"
 "math/big"
 "net"
 "net/http"
 "os"
 "path/filepath"
 "regexp"
 "sort"
 "strings"
 "sync"
 "time"
)

var (
 deviceIDPattern = regexp.MustCompile("^[a-f0-9]{32}$")
 deviceNumberPattern = regexp.MustCompile("^[0-9]{9}$")
 pairRequestPattern = regexp.MustCompile("^[a-f0-9]{32}$")
)

const (
 pairingWindowTTL = 120 * time.Second
 deviceOnlineTTL = 60 * time.Second
 pairingRequestTTL = 120 * time.Second
 pairingRateWindow = 10 * time.Minute
 pairingRateLimit = 5
)

type registeredDevice struct {
 ID string `json:"id"`
 Number string `json:"number"`
 Name string `json:"name"`
 IdentityPublicKey string `json:"identity_public_key"`
 LastSeen time.Time `json:"last_seen"`
 PairingPublicKey string `json:"-"`
 PairingSignature string `json:"-"`
 PairingExpiresAt time.Time `json:"-"`
}

type persistedDevice struct {
 ID string `json:"id"`
 Number string `json:"number"`
 Name string `json:"name"`
 IdentityPublicKey string `json:"identity_public_key"`
 LastSeen time.Time `json:"last_seen"`
}

type pairingRequest struct {
 ID string `json:"id"`
 DeviceID string `json:"device_id"`
 DeviceNumber string `json:"device_number"`
 ViewerPublicKey string `json:"viewer_public_key"`
 Nonce string `json:"nonce"`
 Proof string `json:"proof"`
 Status string `json:"status"`
 CreatedAt time.Time `json:"created_at"`
 ExpiresAt time.Time `json:"expires_at"`
}

type pairingService struct {
 mu sync.Mutex
 token string
 path string
 devices map[string]*registeredDevice
 numbers map[string]string
 requests map[string]*pairingRequest
 attempts map[string][]time.Time
}

func newPairingService(path, token string) (*pairingService, error) {
 if len(token) < 32 { return nil, errors.New("pairing API requires a relay token of at least 32 characters") }
 if path == "" { path = filepath.Join("data", "devices.json") }
 p := &pairingService{token:token,path:path,devices:map[string]*registeredDevice{},numbers:map[string]string{},requests:map[string]*pairingRequest{},attempts:map[string][]time.Time{}}
 if err := p.load(); err != nil { return nil, err }
 return p,nil
}

func (p *pairingService) load() error {
 data,err:=os.ReadFile(p.path)
 if errors.Is(err,os.ErrNotExist) { return nil }
 if err!=nil { return fmt.Errorf("read device registry: %w",err) }
 if len(data)>8<<20 { return errors.New("device registry exceeds 8 MiB") }
 var rows []persistedDevice
 if err=json.Unmarshal(data,&rows);err!=nil { return fmt.Errorf("decode device registry: %w",err) }
 for _,row:=range rows {
  if !deviceIDPattern.MatchString(row.ID)||!deviceNumberPattern.MatchString(row.Number)||!validPublicKey(row.IdentityPublicKey)||p.devices[row.ID]!=nil||p.numbers[row.Number]!="" { return errors.New("device registry contains invalid or duplicate record") }
  p.devices[row.ID]=&registeredDevice{ID:row.ID,Number:row.Number,Name:row.Name,IdentityPublicKey:row.IdentityPublicKey,LastSeen:row.LastSeen}
  p.numbers[row.Number]=row.ID
 }
 return nil
}

func (p *pairingService) persistLocked() error {
 rows:=make([]persistedDevice,0,len(p.devices))
 for _,d:=range p.devices { rows=append(rows,persistedDevice{ID:d.ID,Number:d.Number,Name:d.Name,IdentityPublicKey:d.IdentityPublicKey,LastSeen:d.LastSeen}) }
 sort.Slice(rows,func(i,j int)bool{return rows[i].Number<rows[j].Number})
 data,err:=json.MarshalIndent(rows,"","  ");if err!=nil{return err}
 if err=os.MkdirAll(filepath.Dir(p.path),0700);err!=nil{return err}
 tmp:=p.path+".tmp";if err=os.WriteFile(tmp,data,0600);err!=nil{return err}
 if err=os.Rename(tmp,p.path);err!=nil{_ = os.Remove(tmp);return err}
 return nil
}

func randomDeviceNumber()(string,error){
 n,err:=rand.Int(rand.Reader,big.NewInt(900000000));if err!=nil{return "",err}
 return fmt.Sprintf("%09d",n.Int64()+100000000),nil
}
func randomID()(string,error){var b [16]byte;if _,err:=rand.Read(b[:]);err!=nil{return "",err};return hex.EncodeToString(b[:]),nil}
func decodeKey(v string,n int)([]byte,bool){b,e:=base64.RawURLEncoding.DecodeString(v);return b,e==nil&&len(b)==n}
func validPublicKey(v string)bool{_,ok:=decodeKey(v,ed25519.PublicKeySize);return ok}

func writeJSON(w http.ResponseWriter,status int,value any){w.Header().Set("Content-Type","application/json");w.Header().Set("Cache-Control","no-store");w.WriteHeader(status);_ = json.NewEncoder(w).Encode(value)}
func writeAPIError(w http.ResponseWriter,status int,message string){writeJSON(w,status,map[string]string{"error":message})}
func decodeJSON(w http.ResponseWriter,r *http.Request,dst any)bool{
 r.Body=http.MaxBytesReader(w,r.Body,8192);dec:=json.NewDecoder(r.Body);dec.DisallowUnknownFields()
 if err:=dec.Decode(dst);err!=nil{writeAPIError(w,400,"invalid JSON request");return false}
 if err:=dec.Decode(&struct{}{});err!=io.EOF{writeAPIError(w,400,"request must contain one JSON object");return false}
 return true
}
func (p *pairingService) authorize(w http.ResponseWriter,r *http.Request)bool{
 h:=r.Header.Get("Authorization");if !strings.HasPrefix(h,"Bearer "){writeAPIError(w,401,"unauthorized");return false}
 got:=strings.TrimPrefix(h,"Bearer ");if len(got)!=len(p.token)||subtle.ConstantTimeCompare([]byte(got),[]byte(p.token))!=1{writeAPIError(w,401,"unauthorized");return false}
 return true
}

func (p *pairingService) register(w http.ResponseWriter,r *http.Request){
 if r.Method!=http.MethodPost{writeAPIError(w,405,"method not allowed");return};if !p.authorize(w,r){return}
 var in struct{DeviceID string `json:"device_id"`;Name string `json:"name"`;IdentityPublicKey string `json:"identity_public_key"`}
 if !decodeJSON(w,r,&in){return};in.DeviceID=strings.ToLower(strings.TrimSpace(in.DeviceID));in.Name=strings.TrimSpace(in.Name)
 if !deviceIDPattern.MatchString(in.DeviceID)||in.Name==""||len(in.Name)>48||!validPublicKey(in.IdentityPublicKey){writeAPIError(w,400,"invalid device registration");return}
 p.mu.Lock();defer p.mu.Unlock()
 if d:=p.devices[in.DeviceID];d!=nil{
  if d.IdentityPublicKey!=in.IdentityPublicKey{writeAPIError(w,409,"device identity key changed; rotate device identity");return}
  d.Name=in.Name;d.LastSeen=time.Now().UTC()
  if err:=p.persistLocked();err!=nil{writeAPIError(w,500,"could not persist device registry");return}
  writeJSON(w,200,map[string]string{"device_id":d.ID,"device_number":d.Number});return
 }
 var number string;var err error
 for i:=0;i<20;i++{number,err=randomDeviceNumber();if err!=nil{break};if p.numbers[number]==""{break};number=""}
 if err!=nil||number==""{writeAPIError(w,503,"could not allocate device number");return}
 d:=&registeredDevice{ID:in.DeviceID,Number:number,Name:in.Name,IdentityPublicKey:in.IdentityPublicKey,LastSeen:time.Now().UTC()}
 p.devices[d.ID]=d;p.numbers[number]=d.ID
 if err=p.persistLocked();err!=nil{delete(p.devices,d.ID);delete(p.numbers,number);writeAPIError(w,500,"could not persist device registry");return}
 writeJSON(w,201,map[string]string{"device_id":d.ID,"device_number":d.Number})
}

func (p *pairingService) heartbeat(w http.ResponseWriter,r *http.Request){
 if r.Method!=http.MethodPost{writeAPIError(w,405,"method not allowed");return};if !p.authorize(w,r){return}
 var in struct{DeviceID string `json:"device_id"`};if !decodeJSON(w,r,&in){return}
 p.mu.Lock();defer p.mu.Unlock();d:=p.devices[in.DeviceID];if d==nil{writeAPIError(w,404,"device not registered");return}
 d.LastSeen=time.Now().UTC();if err:=p.persistLocked();err!=nil{writeAPIError(w,500,"could not persist heartbeat");return};writeJSON(w,200,map[string]bool{"ok":true})
}

func pairingWindowMessage(number,expiry,publicKey string)[]byte{return []byte("p2p-pairing-window-v1|"+number+"|"+expiry+"|"+publicKey)}
func (p *pairingService) publishWindow(w http.ResponseWriter,r *http.Request){
 if r.Method!=http.MethodPost{writeAPIError(w,405,"method not allowed");return};if !p.authorize(w,r){return}
 var in struct{DeviceID string `json:"device_id"`;PairingPublicKey string `json:"pairing_public_key"`;Signature string `json:"signature"`;ExpiresAt int64 `json:"expires_at"`}
 if !decodeJSON(w,r,&in){return};if _,ok:=decodeKey(in.PairingPublicKey,32);!ok{writeAPIError(w,400,"invalid pairing public key");return}
 sig,ok:=decodeKey(in.Signature,ed25519.SignatureSize);if !ok{writeAPIError(w,400,"invalid pairing signature");return}
 now:=time.Now();expiry:=time.Unix(in.ExpiresAt,0);if !expiry.After(now)||expiry.After(now.Add(pairingWindowTTL+5*time.Second)){writeAPIError(w,400,"pairing expiry must be within 120 seconds");return}
 p.mu.Lock();defer p.mu.Unlock();d:=p.devices[in.DeviceID];if d==nil{writeAPIError(w,404,"device not registered");return}
 identity,_:=decodeKey(d.IdentityPublicKey,ed25519.PublicKeySize)
 if !ed25519.Verify(ed25519.PublicKey(identity),pairingWindowMessage(d.Number,fmt.Sprint(in.ExpiresAt),in.PairingPublicKey),sig){writeAPIError(w,401,"invalid pairing window signature");return}
 d.PairingPublicKey=in.PairingPublicKey;d.PairingSignature=in.Signature;d.PairingExpiresAt=expiry;d.LastSeen=now.UTC();writeJSON(w,200,map[string]bool{"ok":true})
}

func (p *pairingService) deviceRoute(w http.ResponseWriter,r *http.Request){
 if !p.authorize(w,r){return};number:=strings.TrimPrefix(r.URL.Path,"/api/v1/devices/")
 if !deviceNumberPattern.MatchString(number){writeAPIError(w,404,"device not found");return};if r.Method!=http.MethodGet{writeAPIError(w,405,"method not allowed");return}
 p.mu.Lock();id:=p.numbers[number];d:=p.devices[id];if d==nil{p.mu.Unlock();writeAPIError(w,404,"device not found");return}
 result:=map[string]any{"device_number":d.Number,"name":d.Name,"online":time.Since(d.LastSeen)<=deviceOnlineTTL,"identity_public_key":d.IdentityPublicKey,"pairing_public_key":d.PairingPublicKey,"pairing_signature":d.PairingSignature,"pairing_expires_at":d.PairingExpiresAt.Unix()}
 p.mu.Unlock();writeJSON(w,200,result)
}

func clientHost(r *http.Request)string{h,_,err:=net.SplitHostPort(r.RemoteAddr);if err!=nil{return r.RemoteAddr};return h}
func (p *pairingService) allowPairAttempt(key string,now time.Time)bool{
 history:=p.attempts[key][:0];for _,at:=range p.attempts[key]{if now.Sub(at)<pairingRateWindow{history=append(history,at)}}
 if len(history)>=pairingRateLimit{p.attempts[key]=history;return false};p.attempts[key]=append(history,now);return true
}
func (p *pairingService) createRequest(w http.ResponseWriter,r *http.Request){
 if r.Method!=http.MethodPost{writeAPIError(w,405,"method not allowed");return};if !p.authorize(w,r){return}
 var in struct{DeviceNumber string `json:"device_number"`;ViewerPublicKey string `json:"viewer_public_key"`;Nonce string `json:"nonce"`;Proof string `json:"proof"`}
 if !decodeJSON(w,r,&in){return};if !deviceNumberPattern.MatchString(in.DeviceNumber){writeAPIError(w,400,"invalid device number");return}
 if _,ok:=decodeKey(in.ViewerPublicKey,32);!ok{writeAPIError(w,400,"invalid viewer public key");return}
 if _,ok:=decodeKey(in.Nonce,16);!ok{writeAPIError(w,400,"invalid pairing nonce");return}
 if _,ok:=decodeKey(in.Proof,32);!ok{writeAPIError(w,400,"invalid pairing proof");return}
 now:=time.Now();p.mu.Lock();defer p.mu.Unlock();id:=p.numbers[in.DeviceNumber];d:=p.devices[id]
 if d==nil||now.Sub(d.LastSeen)>deviceOnlineTTL||d.PairingPublicKey==""||!d.PairingExpiresAt.After(now){writeAPIError(w,404,"device is offline or not accepting pairing requests");return}
 rateKey:=clientHost(r)+"|"+in.DeviceNumber;if !p.allowPairAttempt(rateKey,now){writeAPIError(w,429,"too many pairing attempts; wait 10 minutes");return}
 pending:=0;for _,req:=range p.requests{if req.DeviceID==id&&req.Status=="pending"&&req.ExpiresAt.After(now){pending++}}
 if pending>=5{writeAPIError(w,429,"too many pending pairing requests");return}
 requestID,err:=randomID();if err!=nil{writeAPIError(w,500,"could not create pairing request");return}
 req:=&pairingRequest{ID:requestID,DeviceID:id,DeviceNumber:in.DeviceNumber,ViewerPublicKey:in.ViewerPublicKey,Nonce:in.Nonce,Proof:in.Proof,Status:"pending",CreatedAt:now.UTC(),ExpiresAt:now.Add(pairingRequestTTL).UTC()}
 p.requests[requestID]=req;writeJSON(w,201,map[string]any{"request_id":requestID,"expires_at":req.ExpiresAt.Unix()})
}
func (p *pairingService) listRequests(w http.ResponseWriter,r *http.Request){
 if r.Method!=http.MethodGet{writeAPIError(w,405,"method not allowed");return};if !p.authorize(w,r){return}
 id:=r.URL.Query().Get("device_id");p.mu.Lock();defer p.mu.Unlock();if p.devices[id]==nil{writeAPIError(w,404,"device not registered");return}
 now:=time.Now();result:=make([]pairingRequest,0)
 for _,req:=range p.requests{if req.DeviceID==id&&req.Status=="pending"&&req.ExpiresAt.After(now){result=append(result,*req)}}
 sort.Slice(result,func(i,j int)bool{return result[i].CreatedAt.Before(result[j].CreatedAt)});if len(result)>20{result=result[:20]}
 writeJSON(w,200,map[string]any{"requests":result})
}
func (p *pairingService) decideRequest(w http.ResponseWriter,r *http.Request){
 if r.Method!=http.MethodPost{writeAPIError(w,405,"method not allowed");return};if !p.authorize(w,r){return}
 id:=strings.TrimSuffix(strings.TrimPrefix(r.URL.Path,"/api/v1/pairing/requests/"),"/decision");if !pairRequestPattern.MatchString(id){writeAPIError(w,404,"pairing request not found");return}
 var in struct{DeviceID string `json:"device_id"`;Approved bool `json:"approved"`};if !decodeJSON(w,r,&in){return}
 p.mu.Lock();defer p.mu.Unlock();req:=p.requests[id]
 if req==nil||!req.ExpiresAt.After(time.Now())||req.Status!="pending"{writeAPIError(w,404,"pairing request expired or already handled");return}
 if req.DeviceID!=in.DeviceID{writeAPIError(w,403,"pairing request belongs to another device");return}
 if in.Approved{req.Status="approved";if d:=p.devices[in.DeviceID];d!=nil{d.PairingPublicKey="";d.PairingSignature="";d.PairingExpiresAt=time.Time{}}
  for otherID,other:=range p.requests{if otherID!=id&&other.DeviceID==in.DeviceID&&other.Status=="pending"{other.Status="expired"}}
 }else{req.Status="denied"}
 writeJSON(w,200,map[string]string{"status":req.Status})
}
func (p *pairingService) requestStatus(w http.ResponseWriter,r *http.Request){
 if r.Method!=http.MethodGet{writeAPIError(w,405,"method not allowed");return};if !p.authorize(w,r){return}
 id:=strings.TrimPrefix(r.URL.Path,"/api/v1/pairing/requests/");if !pairRequestPattern.MatchString(id){writeAPIError(w,404,"pairing request not found");return}
 p.mu.Lock();req:=p.requests[id];if req==nil{p.mu.Unlock();writeAPIError(w,404,"pairing request not found");return}
 status:=req.Status;if !req.ExpiresAt.After(time.Now())&&status=="pending"{status="expired";req.Status=status}
 result:=map[string]any{"status":status,"expires_at":req.ExpiresAt.Unix()}
 if d:=p.devices[req.DeviceID];d!=nil{result["device_number"]=d.Number;result["identity_public_key"]=d.IdentityPublicKey;result["pairing_public_key"]=d.PairingPublicKey;result["pairing_signature"]=d.PairingSignature}
 p.mu.Unlock();writeJSON(w,200,result)
}
func (p *pairingService) handler()http.Handler{
 mux:=http.NewServeMux()
 mux.HandleFunc("/api/v1/devices/register",p.register)
 mux.HandleFunc("/api/v1/devices/heartbeat",p.heartbeat)
 mux.HandleFunc("/api/v1/devices/pairing-window",p.publishWindow)
 mux.HandleFunc("/api/v1/devices/",p.deviceRoute)
 mux.HandleFunc("/api/v1/pairing/requests",p.listRequests)
 mux.HandleFunc("/api/v1/pairing/requests/",func(w http.ResponseWriter,r *http.Request){if strings.HasSuffix(r.URL.Path,"/decision"){p.decideRequest(w,r)}else{p.requestStatus(w,r)}})
 return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
  if r.TLS == nil && r.Header.Get("X-Forwarded-Proto") != "https" && !isLocal(r.RemoteAddr) {
   writeAPIError(w, http.StatusUpgradeRequired, "TLS required")
   return
  }
  mux.ServeHTTP(w, r)
 })
}
