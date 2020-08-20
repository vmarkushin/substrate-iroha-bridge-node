#!/usr/bin/env bash

set -e

echo "*** Start Substrate Iroha Bridge ***"

cd $(dirname ${BASH_SOURCE[0]})/../../..

docker-compose -f docker-compose-substrate.yml down --remove-orphans
docker-compose -f docker-compose-substrate.yml run --rm --service-ports dev-manual $@