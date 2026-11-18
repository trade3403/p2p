FROM golang:1.23-alpine AS builder
WORKDIR /src
COPY go.mod ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -ldflags='-s -w' -o /relay .
FROM alpine:3.21
RUN adduser -D -u 10001 app && mkdir -p /data && chown app:app /data
USER app
COPY --from=builder /relay /relay
EXPOSE 8080
EXPOSE 3478/udp
ENTRYPOINT ["/relay"]
