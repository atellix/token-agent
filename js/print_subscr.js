const { PublicKey } = require('@solana/web3.js')
const anchor = require('@project-serum/anchor')
const provider = anchor.AnchorProvider.env()
anchor.setProvider(provider)

const tokenAgent = anchor.workspace.TokenAgent

async function main() {
    var subscrData = new PublicKey('4xFJN6iE7wAfc57aZ7iqVr88aqo1c6nvTdXZrjz7Yujj')
    var act = await tokenAgent.account.subscrData.fetch(subscrData)
    console.log(act)
}

main()
