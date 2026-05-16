# Zallet Parity Report

- **Total Tests**: 24
- **✅ Matches**: 2
- **❌ Diffs**: 1
- **📋 Expected Diffs**: 1
- **🔍 Missing**: 19
- **📋 Expected Missing**: 0
- **⚠️ Errors**: 1

## Detailed Results

| Method | Status | Details |
| :--- | :--- | :--- |
| `getaddressesbyaccount` | 🔍 Missing | Method `getaddressesbyaccount` not found on one endpoint |
| `getbalance` | 🔍 Missing | Method `getbalance` not found on one endpoint |
| `getbestblockhash` | 🔍 Missing | Method `getbestblockhash` not found on one endpoint |
| `getblockchaininfo` | ❌ Diff | 1 field(s) differ: `/chain` |
| `getblockcount` | ✅ Match |  |
| `getconnectioncount` | ✅ Match |  |
| `getdeprecationinfo` | 🔍 Missing | Method `getdeprecationinfo` not found on one endpoint |
| `getdifficulty` | 🔍 Missing | Method `getdifficulty` not found on one endpoint |
| `getmemoryinfo` | ⚠️ Error | Upstream error: jsonrpsee error: ErrorObject { code: InternalError, message: "Internal error", data: None } |
| `getmininginfo` | 🔍 Missing | Method `getmininginfo` not found on one endpoint |
| `getnettotals` | 🔍 Missing | Method `getnettotals` not found on one endpoint |
| `getnetworkinfo` | 📋 Expected Diff | 1 field(s): `/useragent` — _Zallet reports its own agent/version string (e.g. 'Zallet/0.1.0') while zcashd reports '/MagicBean:...' — intentional product identity difference._  |
| `getwalletinfo` | 🔍 Missing | Method `getwalletinfo` not found on one endpoint |
| `listaddresses` | 🔍 Missing | Method `listaddresses` not found on one endpoint |
| `listreceivedbyaddress` | 🔍 Missing | Method `listreceivedbyaddress` not found on one endpoint |
| `listtransactions` | 🔍 Missing | Method `listtransactions` not found on one endpoint |
| `listunspent` | 🔍 Missing | Method `listunspent` not found on one endpoint |
| `validateaddress` | 🔍 Missing | Method `validateaddress` not found on one endpoint |
| `z_getoperationresult` | 🔍 Missing | Method `z_getoperationresult` not found on one endpoint |
| `z_getoperationstatus` | 🔍 Missing | Method `z_getoperationstatus` not found on one endpoint |
| `z_gettotalbalance` | 🔍 Missing | Method `z_gettotalbalance` not found on one endpoint |
| `z_listaddresses` | 🔍 Missing | Method `z_listaddresses` not found on one endpoint |
| `z_listunspent` | 🔍 Missing | Method `z_listunspent` not found on one endpoint |
| `z_validateaddress` | 🔍 Missing | Method `z_validateaddress` not found on one endpoint |
