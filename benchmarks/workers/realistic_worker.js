async function backgroundTask(env) {
  const min_work_factor = env.USER_CONFIG?.min_cpu_work_factor || 10_000;
  const max_work_factor = env.USER_CONFIG?.max_cpu_work_factor || 100_000;
  const min_sleep_ms = env.USER_CONFIG?.min_sleep_ms || 50;
  const max_sleep_ms = env.USER_CONFIG?.max_sleep_ms || 1000;

  function getRandomInt(min, max) {
    return Math.floor(Math.random() * (max - min + 1)) + min;
  }

  function isPrime(n) {
    if (n < 2) return false;
    for (let i = 2; i * i <= n; i++) {
      if (n % i === 0) return false;
    }
    return true;
  }

  function fibonacciIterative(n) {
    if (n <= 1) return n;
    let a = 0,
      b = 1;
    for (let i = 2; i <= n; i++) {
      let temp = a + b;
      a = b;
      b = temp;
    }
    return b;
  }

  function matrixMultiply(size) {
    const a = Array(size)
      .fill()
      .map(() => Array(size).fill(Math.random()));
    const b = Array(size)
      .fill()
      .map(() => Array(size).fill(Math.random()));
    const result = Array(size)
      .fill()
      .map(() => Array(size).fill(0));
    for (let i = 0; i < size; i++) {
      for (let j = 0; j < size; j++) {
        for (let k = 0; k < size; k++) {
          result[i][j] += a[i][k] * b[k][j];
        }
      }
    }
    return result;
  }

  function arraySort(size) {
    const arr = Array(size)
      .fill()
      .map(() => Math.floor(Math.random() * 10000));
    return arr.sort((a, b) => a - b);
  }

  const computationTasks = [
    (workFactor) => {
      const startNum = workFactor * 10;
      const endNum = startNum + 2000;
      for (let i = startNum; i < endNum; i++) {
        isPrime(i);
      }
    },
    (workFactor) => {
      const numCalculations = Math.floor(workFactor / 5);
      for (let i = 0; i < numCalculations; i++) {
        fibonacciIterative(35);
      }
    },
    (workFactor) => {
      const matrixSize = Math.min(10 + Math.floor(workFactor / 500), 120);
      if (matrixSize > 5) {
        matrixMultiply(matrixSize);
      }
    },
    (workFactor) => {
      const arraySize = Math.min(workFactor * 10, 500_000);
      arraySort(arraySize);
    },
    (workFactor) => {
      const opCount = workFactor;
      let str = "benchmark";
      for (let i = 0; i < opCount; i++) {
        str = str.split("").reverse().join("") + i.toString();
        str = str.substring(0, 30);
      }
    },
  ];

  console.log("Starting realistic background task...");

  while (true) {
    const workFactor = getRandomInt(min_work_factor, max_work_factor);
    const taskIndex = getRandomInt(0, computationTasks.length - 1);
    const startTime = Date.now();

    console.log(
      `Starting computation task ${taskIndex + 1} with a workFactor of ${workFactor}`,
    );
    computationTasks[taskIndex](workFactor);

    const computationTime = Date.now() - startTime;
    console.log(`Computation completed in ${computationTime}ms`);

    const sleepTime = getRandomInt(min_sleep_ms, max_sleep_ms);
    console.log(`Sleeping for ${sleepTime}ms`);

    await new Promise((resolve) => setTimeout(resolve, sleepTime));
  }
}

async function handleLaunch(eventPayload, env) {
  backgroundTask(env).catch((error) => {
    console.error("Background task error:", error);
  });
  return new Response(
    "Realistic worker launched with scalable computation and sleep cycles.",
    { status: 200 },
  );
}

async function handleStatus(eventPayload, env) {
  return new Response(
    "Worker is running background simulation with random workloads.",
    { status: 200 },
  );
}

const handlers = {
  launch: handleLaunch,
  status: handleStatus,
};

export default {
  async fetch(request, env, ctx) {
    const vmInvocationPayload = await request.json();
    const handler = handlers[vmInvocationPayload.handler];

    if (handler) {
      return handler(vmInvocationPayload.event_payload, env);
    } else {
      return new Response(`No handler for ${vmInvocationPayload.handler}`, {
        status: 404,
      });
    }
  },
};
