export default async function handler(req, res) {
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
  res.setHeader("Access-Control-Allow-Headers", "Content-Type");

  if (req.method === "OPTIONS") {
    return res.status(200).end();
  }

  const sigs = req.body && req.body.signatures
    ? req.body.signatures
    : [
        "56aLprvPNEnHSMZumQxLD8rmHoE1DTpjdRKMUStaePn6nr8p9cQWqSRCs1L1r6oFE2K1WX6YGqwQXMaMMMo1aGEM",
        "5eWQX7EcKncuh9ztWzGPxck6xc6fXezp63YxuvnVtK63eCbUoCnYFSe3JitVpNDBgiNtJMJeuzmgDHBCsjSxKCAR",
        "3Mk2Cm32pnbtLNARCDjFx4evWQ9ti4XP24rGRcbkc9cvD55HJJsmK3Sd4X9fDx7vFvdp33Bm6LNmgq81KsUFa9a1",
        "2FEkYPk9w9APnGMVLSYSkQMEWFFY247WSQzD3dSkju9dkdeC6x7BCDKwKgunsTPxBZHrrmQKNsaJiz2k7mK3zruE",
        "368DBQqxJCZQDc7LdqnzS81Z816Xj9Qk2m7f7kY1zRk4b",
        "2LU7cdiKRXn2tFsSfds1k7k2mQz3W9Y2b9YmQz3W9Y2b",
        "3uRPuB7KmaJBxm8DUgWJk7k2mQz3W9Y2b9YmQz3W9Y2b"
      ];

  try {
    const rpcResp = await fetch("https://api.mainnet-beta.solana.com", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "User-Agent": "SolanaValidatorJudgeVerifier/1.0"
      },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "getSignatureStatuses",
        params: [sigs, { searchTransactionHistory: true }]
      })
    });
    const data = await rpcResp.json();
    return res.status(200).json(data);
  } catch (err) {
    return res.status(500).json({ error: err.message });
  }
}
