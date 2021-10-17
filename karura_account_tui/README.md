This small Rust project performs a GraphQL query on the Karura SubQL project. Given an account id, it will pull the total, median, and mean KAR in transfers sent and received for that account.

# Example
```
cargo run oa4fpTrwnXsPR1gvfUpHZKpyrPY1EReb4RPnKuXhcaPpCMh
```
will output:
```
Transfer Summary Statistics For AccountId oa4fpTrwnXsPR1gvfUpHZKpyrPY1EReb4RPnKuXhcaPpCMh
--------------------------------------------
Transfers Received| Total:0 KAR, Median:0 KAR, Mean:0 KAR
Transfers Sent| Total:289749500000000 KAR, Median:4974750000000 KAR, Mean:28974950000000 KAR
Net Transfer Amount: -289749500000000 KAR
```
