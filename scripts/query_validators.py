import urllib.request, json

req = urllib.request.Request('https://api.mainnet-beta.solana.com', data=json.dumps({'jsonrpc':'2.0','id':1,'method':'getVoteAccounts'}).encode(), headers={'Content-Type':'application/json'})
votes = json.loads(urllib.request.urlopen(req).read().decode())['result']['current']
req_nodes = urllib.request.Request('https://api.mainnet-beta.solana.com', data=json.dumps({'jsonrpc':'2.0','id':1,'method':'getClusterNodes'}).encode(), headers={'Content-Type':'application/json'})
nodes = {n['pubkey']: n for n in json.loads(urllib.request.urlopen(req_nodes).read().decode())['result']}

client_stakes = {}
top_by_client = {}
for v in votes:
    node_pk = v['nodePubkey']
    node = nodes.get(node_pk, {})
    cid = node.get('clientId', 'Unknown')
    stake = v['activatedStake'] / 1e9
    client_stakes[cid] = client_stakes.get(cid, 0) + stake
    if cid not in top_by_client:
        top_by_client[cid] = []
    top_by_client[cid].append({
        'node': node_pk,
        'vote': v['votePubkey'],
        'stake': stake,
        'version': node.get('version'),
        'tpuQuic': node.get('tpuQuic') or node.get('tpu')
    })

print("Stake distribution by client:")
for cid, stake in sorted(client_stakes.items(), key=lambda x: -x[1]):
    print(f"  {cid}: {stake:,.0f} SOL")

print("\nTop validators by client:")
for cid in ['Agave', 'JitoLabs', 'Firedancer', 'HarmonicFiredancer', 'AgaveBam', 'FireBAM']:
    if cid in top_by_client:
        top = sorted(top_by_client[cid], key=lambda x: -x['stake'])[:3]
        print(f"--- {cid} ---")
        for val in top:
            print(f"  Node: {val['node']}, Stake: {val['stake']:,.0f} SOL, Ver: {val['version']}, TPU: {val['tpuQuic']}")
