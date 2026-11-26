// Enumerate local, usable IPv4 unicast addresses without DNS or probes.
#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#include <iphlpapi.h>
#include <cstdint>
#include <vector>

extern "C" uint32_t p2p_lan_ipv4_candidates(uint32_t* output, uint32_t capacity) {
    if (!output || !capacity) return 0;
    try {
        ULONG size = 15 * 1024;
        std::vector<unsigned char> buffer(size);
        constexpr ULONG flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST |
            GAA_FLAG_SKIP_DNS_SERVER | GAA_FLAG_SKIP_FRIENDLY_NAME;
        ULONG result = GetAdaptersAddresses(AF_INET, flags, nullptr,
            reinterpret_cast<IP_ADAPTER_ADDRESSES*>(buffer.data()), &size);
        if (result == ERROR_BUFFER_OVERFLOW && size <= 1024 * 1024) {
            buffer.resize(size);
            result = GetAdaptersAddresses(AF_INET, flags, nullptr,
                reinterpret_cast<IP_ADAPTER_ADDRESSES*>(buffer.data()), &size);
        }
        if (result != NO_ERROR) return 0;
        uint32_t count = 0;
        for (auto* adapter = reinterpret_cast<IP_ADAPTER_ADDRESSES*>(buffer.data());
             adapter && count < capacity; adapter = adapter->Next) {
            if (adapter->OperStatus != IfOperStatusUp ||
                adapter->IfType == IF_TYPE_SOFTWARE_LOOPBACK ||
                adapter->IfType == IF_TYPE_TUNNEL) continue;
            for (auto* unicast = adapter->FirstUnicastAddress;
                 unicast && count < capacity; unicast = unicast->Next) {
                auto* addr = unicast->Address.lpSockaddr;
                if (!addr || addr->sa_family != AF_INET ||
                    unicast->Address.iSockaddrLength < sizeof(sockaddr_in) ||
                    unicast->DadState != IpDadStatePreferred) continue;
                const uint32_t ip = ntohl(reinterpret_cast<sockaddr_in*>(addr)->sin_addr.s_addr);
                const uint32_t a = ip >> 24, b = (ip >> 16) & 255, d = ip & 255;
                if (!(a == 10 || (a == 172 && b >= 16 && b <= 31) ||
                    (a == 192 && b == 168)) || d == 0 || d == 255) continue;
                bool seen = false;
                for (uint32_t i = 0; i < count; ++i) if (output[i] == ip) seen = true;
                if (!seen) output[count++] = ip;
            }
        }
        return count;
    } catch (...) {
        return 0;
    }
}
