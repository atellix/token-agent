const { PublicKey } = require('@solana/web3.js')
const anchor = require('@project-serum/anchor')
const provider = anchor.Provider.env()
anchor.setProvider(provider)

const tokenAgent = anchor.workspace.TokenAgent

async function main() {
    var subscrData = new PublicKey('9tZurQLMZCfoQVt74KVGUzZcYtyDitozFoqVabb2AKvu')
    var act = await tokenAgent.account.subscrData.fetch(subscrData)
    console.log(act)
}

main()
