use std::net::{IpAddr, Ipv6Addr};
use reqwest::{redirect, Client};
use cidr::Ipv6Cidr;
use rand::random;

pub fn get_rand_ipv6(subnet: &str) -> IpAddr {
    let (ipv6, prefix_len) = match subnet.parse::<Ipv6Cidr>() {
        Ok(cidr) => {
            let ipv6 = cidr.first_address();
            let length = cidr.network_length();
            (ipv6, length)
        }
        Err(_) => {
            panic!("invalid IPv6 subnet");
        }
    };

    let ipv6_u128: u128 = u128::from(ipv6);
    let rand: u128 = random();

    let net_part = (ipv6_u128 >> (128 - prefix_len)) << (128 - prefix_len);
    let host_part = (rand << prefix_len) >> prefix_len;
    let result = net_part | host_part;

    IpAddr::V6(Ipv6Addr::from(result))
}

pub fn create_client(subnet: Option<&str>, user_agent: &str) -> Client {
    if let Some(subnet) = subnet {
        let ip = get_rand_ipv6(subnet);

        Client::builder()
            .redirect(redirect::Policy::none())
            .danger_accept_invalid_certs(true)
            .user_agent(user_agent)
            .local_address(Some(ip))
            .build().unwrap()
    } else {
        Client::builder()
            .redirect(redirect::Policy::none())
            .danger_accept_invalid_certs(true)
            .user_agent(user_agent)
            .build().unwrap()
    }
}