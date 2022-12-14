const hardhat = require("hardhat");
const { expect } = require("chai")
const { getCallRevertReason } = require("./common")

describe("Governance unit tests", function () {
    this.timeout(50000);

    let testContract
    before(async () => {
        let governanceAddressDeployed;
        const contractFactory = await hardhat.ethers.getContractFactory("Governance");
        testContract = await contractFactory.deploy();
        await testContract.initialize(ethers.utils.defaultAbiCoder.encode(["address"], [await testContract.signer.getAddress()]));
    });

    it("checking correctness of using MAX_AMOUNT_OF_REGISTERED_TOKENS constant", async () => {
        const MAX_AMOUNT_OF_REGISTERED_TOKENS = 5;
        for (let step = 1; step <= MAX_AMOUNT_OF_REGISTERED_TOKENS + 1; step++) {
            let {revertReason} = await getCallRevertReason( () => testContract.addToken("0x" + step.toString().padStart(40, '0')) )
            if (step !== MAX_AMOUNT_OF_REGISTERED_TOKENS + 1) {
                expect(revertReason).equal("VM did not revert")
            } else{
                expect(revertReason).not.equal("VM did not revert")
            }
        }
    });

});
