const { expect } = require("chai")
const { getCallRevertReason } = require("./common")
const hardhat = require("hardhat");


describe("Ownable unit tests", function () {
    this.timeout(50000);

    let testContract
    let wallet1, wallet2;
    before(async () => {
        [wallet1, wallet2] = await hardhat.ethers.getSigners();
        const contractFactory = await hardhat.ethers.getContractFactory("Ownable");
        testContract = await contractFactory.deploy(wallet1.address);
    });

    it("checking correctness of setting mastership in constructor", async () => {
        expect(await testContract.getMaster()).to.equal(wallet1.address)
    });

    it("checking correctness of transferring mastership to zero address", async () => {
        let {revertReason} = await getCallRevertReason( () => testContract.transferMastership("0x0000000000000000000000000000000000000000", {gasLimit: "300000"}) );
        expect(revertReason).equal("otp11")
    });

    it("checking correctness of transferring mastership", async () => {
        /// transfer mastership to wallet2
        await testContract.transferMastership(wallet2.address);
        expect(await testContract.getMaster()).to.equal(wallet2.address)

        /// try to transfer mastership to wallet1 by wallet1 call
        let {revertReason} = await getCallRevertReason( () => testContract.transferMastership(wallet1.address, {gasLimit: "300000"}) );
        expect(revertReason).equal("oro11")

        /// transfer mastership back to wallet1
        let testContract_with_wallet2_signer = await testContract.connect(wallet2);
        await testContract_with_wallet2_signer.transferMastership(wallet1.address);
        expect(await testContract.getMaster()).to.equal(wallet1.address)
    });

});
