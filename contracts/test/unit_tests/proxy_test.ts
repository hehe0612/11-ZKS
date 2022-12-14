const { expect } = require('chai');
const { getCallRevertReason } = require('./common');
const hardhat = require('hardhat');

import { Contract, constants } from 'ethers';

const TX_OPTS = {
    gasLimit: 300000
};
describe('Proxy unit tests', function () {
    this.timeout(50000);

    let proxyTestContract;
    let dummyFirst;
    let wallet, wallet1;
    before(async () => {
        [wallet, wallet1] = await hardhat.ethers.getSigners();

        const dummyFactory = await hardhat.ethers.getContractFactory('DummyFirst');
        dummyFirst = await dummyFactory.deploy();
        const proxyFactory = await hardhat.ethers.getContractFactory('Proxy');
        proxyTestContract = await proxyFactory.deploy(dummyFirst.address, [1, 2]);
    });

    it('check delegatecall', async () => {
        const dummyFactory = await hardhat.ethers.getContractFactory('DummyFirst');
        const proxyDummyInterface = dummyFactory.attach(proxyTestContract.address);
        expect(await proxyDummyInterface.get_DUMMY_INDEX()).to.equal(1);
    });

    it('checking that requireMaster calls present', async () => {
        const testContract_with_wallet2_signer = await proxyTestContract.connect(wallet1);
        expect(
            (
                await getCallRevertReason(() =>
                    testContract_with_wallet2_signer.upgradeTarget(constants.AddressZero, [], TX_OPTS)
                )
            ).revertReason
        ).equal('oro11');
        expect(
            (await getCallRevertReason(() => testContract_with_wallet2_signer.upgradeNoticePeriodStarted(TX_OPTS)))
                .revertReason
        ).equal('oro11');
        expect(
            (await getCallRevertReason(() => testContract_with_wallet2_signer.upgradePreparationStarted(TX_OPTS)))
                .revertReason
        ).equal('oro11');
        expect(
            (await getCallRevertReason(() => testContract_with_wallet2_signer.upgradeCanceled(TX_OPTS))).revertReason
        ).equal('oro11');
        expect(
            (await getCallRevertReason(() => testContract_with_wallet2_signer.upgradeFinishes(TX_OPTS))).revertReason
        ).equal('oro11');
    });

    it('checking Proxy reverts', async () => {
        expect((await getCallRevertReason(() => proxyTestContract.initialize([], TX_OPTS))).revertReason).equal(
            'ini11'
        );
        expect((await getCallRevertReason(() => proxyTestContract.upgrade([], TX_OPTS))).revertReason).equal('upg11');
        expect(
            (await getCallRevertReason(() => proxyTestContract.upgradeTarget(proxyTestContract.address, [], TX_OPTS)))
                .revertReason
        ).equal('ufu11');
    });
});
