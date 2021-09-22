use ::reqwest::blocking::Client;
use graphql_client::{reqwest::post_graphql_blocking as post_graphql, GraphQLQuery};
use std::error::Error;
use std::env;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/schema.json",
    query_path = "src/transfers_query.graphql",
    response_derives = "Debug",
)]

pub struct TransfersQuery;

fn median(data: &mut Vec<i64>) -> i64 {
    data.sort();
    let mid = data.len() / 2;
    if data.len() % 2 == 0 {
        (data[mid - 1] + data[mid]) / 2
    } else {
        data[mid]
    }
}

fn perform_query(account_id: &String) -> Result<(), Box<dyn Error>> {
    let variables = transfers_query::Variables{
        accountid: account_id.to_string(),
    };

    let client = Client::new();

    let response_body = post_graphql::<TransfersQuery, _>(&client, "https://api.subquery.network/sq/AcalaNetwork/karura", variables).unwrap();
    println!("{:#?}", response_body);

    let response_data = &response_body.data.expect("missing response data");
    let mut transfer_in_amounts: Vec<i64> = Vec::new();
    let mut transfer_out_amounts: Vec<i64> = Vec::new();
    let mut transfer_in_count = 0;
    let mut transfer_out_count = 0;

    //We are only looking at one account, so accounts will only have one account in its nodes vector
    if let Some(n) = &response_data.accounts.as_ref().expect("No transfers for account").nodes.iter().flat_map(|n| n.iter()).next() {
        if n.transfer_in.total_count > 0 {
            transfer_in_amounts = n
            .transfer_in.nodes.iter()
            .flat_map(|n| n.iter())
            //unwrap of amount is ok because we know we have transfers in, so it will return Some
            .map(|t| t.amount.as_ref().unwrap().parse::<i64>().expect("Not parseable as an integer!")).collect();  

            transfer_in_count = n.transfer_in.total_count;
        }

        if n.transfer_out.total_count > 0 {
            transfer_out_amounts = n
            .transfer_out.nodes.iter()
            .flat_map(|n| n.iter())
            .map(|t| t.amount.as_ref().unwrap().parse::<i64>().expect("Not parseable as an integer!")).collect();  

            transfer_out_count = n.transfer_out.total_count;
        }
    }
    
    let transfer_in_sum: i64 = transfer_in_amounts.iter().sum();
    let transfer_out_sum: i64 = transfer_out_amounts.iter().sum();

    

    transfer_in_amounts.sort();
    transfer_out_amounts.sort();

    let mut transfer_in_median: i64 = 0;  
    let mut transfer_out_median: i64 = 0; 

    let mut transfer_in_mean: i64 = 0;
    if transfer_in_count > 0 {
        transfer_in_mean = transfer_in_sum / transfer_in_count;
        transfer_in_median = median(&mut transfer_in_amounts);
    } 

    let mut transfer_out_mean: i64 = 0;
    if transfer_out_count > 0 {
        transfer_out_mean = transfer_out_sum / transfer_out_count;
        transfer_out_median = median(&mut transfer_out_amounts);
    } 

    let transfer_net_sum: i64 = transfer_in_sum - transfer_out_sum;

    println!("Transfer Summary Statistics For AccountId {}", account_id);
    println!("--------------------------------------------");
    println!("Transfers Received| Total:{} KAR, Median:{} KAR, Mean:{} KAR", transfer_in_sum, transfer_in_median, transfer_in_mean);
    println!("Transfers Sent| Total:{} KAR, Median:{} KAR, Mean:{} KAR", transfer_out_sum, transfer_out_median, transfer_out_mean);
    println!("Net Transfer Amount: {} KAR", transfer_net_sum);
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let accountid = &args[1];
    perform_query(accountid);
}
