# Load Testing with K6 Operator

This guide explains how to use the K6 Operator to load test your applications and test the HorizontalPodAutoscaler (HPA) functionality.

## Prerequisites

- Kubernetes cluster with Oxide
- Metrics Server installed (`oxide install-metrics-server`)
- HorizontalPodAutoscaler configured on your deployment

## Install K6 Operator

The k6-operator can be installed using Helm:

```bash
# Add the k6 Helm repository
helm repo add grafana https://grafana.github.io/helm-charts
helm repo update

# Install the k6-operator
helm install k6-operator grafana/k6-operator \
  --namespace k6-operator-system \
  --create-namespace

# Verify installation
kubectl get pods -n k6-operator-system
```

**Alternative: Install with kubectl (without Helm)**

If you prefer not to use Helm, you can install the operator manually:

```bash
# Clone the k6-operator repository
git clone https://github.com/grafana/k6-operator.git
cd k6-operator

# Install CRDs and operator
make deploy

# Verify installation
kubectl get pods -n k6-operator-system
```

## Deploy Test Application (nginx)

First, ensure you have the nginx deployment with HPA configured:

```bash
kubectl apply -f nginx-deployment.yaml
```

This will create:

- Nginx deployment with 3 replicas
- Service exposing nginx
- HorizontalPodAutoscaler (min: 3, max: 100)
- PodDisruptionBudget for high availability

## Run Load Test

```bash
# Apply the K6 load test configuration
kubectl apply -f k6-load-test.yaml

# Watch the test progress
kubectl get k6 -w

# View K6 test logs
kubectl logs -l k6_cr=nginx-load-test -f
```

## Monitor Autoscaling

Open multiple terminal windows to monitor different aspects:

### Terminal 1: Watch Pods

```bash
watch kubectl get pods -n default
```

### Terminal 2: Watch HPA Status

```bash
watch kubectl get hpa nginx-hpa
```

### Terminal 3: Watch Resource Metrics

```bash
# Requires metrics-server
watch kubectl top pods -n default
```

### Terminal 4: Watch K6 Test Progress

```bash
kubectl logs -l k6_cr=nginx-load-test -f
```

## Understanding the Load Test

The K6 test follows this pattern:

```
Stage 1: 30s  - Ramp up to 50 users
Stage 2: 2min - Maintain 50 users
Stage 3: 30s  - Ramp up to 100 users
Stage 4: 2min - Maintain 100 users
Stage 5: 30s  - Ramp up to 200 users
Stage 6: 2min - Maintain 200 users
Stage 7: 30s  - Ramp down to 0 users
```

**Total Duration:** ~7.5 minutes

## Expected HPA Behavior

Based on the nginx HPA configuration (60% CPU target):

1. **Initial State**: 3 pods running
2. **Load Increases**: CPU usage rises above 60%
3. **HPA Scales Up**: More pods are created (up to 100)
4. **Load Stabilizes**: Pods handle requests, CPU drops below 60%
5. **Load Decreases**: CPU usage falls below target
6. **HPA Scales Down**: Pods are removed (down to 3 minimum)

## Viewing Results

### Check HPA Events

```bash
kubectl describe hpa nginx-hpa
```

Look for events like:

```
Normal  SuccessfulRescale  2m   horizontal-pod-autoscaler  New size: 10; reason: cpu resource utilization (percentage of request) above target
Normal  SuccessfulRescale  1m   horizontal-pod-autoscaler  New size: 5; reason: All metrics below target
```

### Check Pod Distribution

```bash
kubectl get pods -o wide | grep nginx
```

### View Metrics History

```bash
# Current CPU/Memory usage
kubectl top pods -l app=nginx

# HPA current metrics
kubectl get hpa nginx-hpa -o yaml | grep -A 10 currentMetrics
```

## Cleanup

```bash
# Delete the K6 test
kubectl delete k6 nginx-load-test

# Delete the ConfigMap
kubectl delete configmap k6-test-script

# (Optional) Remove nginx deployment
kubectl delete -f nginx-deployment.yaml
```

## Advanced: Custom Load Tests

### Create Custom K6 Test Script

```javascript
import http from "k6/http";
import { check, sleep } from "k6";

export let options = {
  stages: [
    { duration: "1m", target: 100 },
    { duration: "5m", target: 100 },
    { duration: "1m", target: 0 },
  ],
};

export default function () {
  let response = http.get("http://your-service.default.svc.cluster.local");
  check(response, {
    "status is 200": (r) => r.status === 200,
  });
  sleep(1);
}
```

### Apply Custom Test

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: my-custom-test
  namespace: default
data:
  test.js: |
    # Your custom test script
---
apiVersion: k6.io/v1alpha1
kind: K6
metadata:
  name: my-load-test
  namespace: default
spec:
  parallelism: 2
  script:
    configMap:
      name: my-custom-test
      file: test.js
```

## Testing Cluster Autoscaler with Load

To test both HPA and Cluster Autoscaler together:

1. **Set aggressive HPA settings**:

   ```yaml
   minReplicas: 3
   maxReplicas: 100
   metrics:
     - type: Resource
       resource:
         name: cpu
         target:
           type: Utilization
           averageUtilization: 50 # Lower threshold
   ```

2. **Configure pod resource requests** to fill nodes quickly:

   ```yaml
   resources:
     requests:
       cpu: 500m # Higher CPU request
       memory: 256Mi
   ```

3. **Run intense load test**:

   - HPA will scale pods up to 100
   - Pods won't fit on existing nodes
   - Cluster Autoscaler will add worker nodes

4. **Monitor both scalers**:

   ```bash
   # Watch HPA
   watch kubectl get hpa

   # Watch nodes (Cluster Autoscaler)
   watch kubectl get nodes

   # Watch autoscaler logs
   kubectl logs -n oxide-system -l app=cluster-autoscaler -f
   ```

## Troubleshooting

### K6 Test Not Starting

**Check K6 Operator logs:**

```bash
kubectl logs -n k6-operator-system -l app.kubernetes.io/name=k6-operator
```

**Verify ConfigMap exists:**

```bash
kubectl get configmap k6-test-script
```

### HPA Not Scaling

**Check metrics server:**

```bash
kubectl top nodes
kubectl top pods
```

**Verify HPA has metrics:**

```bash
kubectl get hpa nginx-hpa
```

If metrics show `<unknown>`, wait 30-60 seconds for metrics to populate.

**Check pod resource requests:**

```bash
kubectl get deployment nginx -o yaml | grep -A 5 resources
```

Pods MUST have resource requests for HPA to work.

### Load Test Failing

**Check service connectivity:**

```bash
kubectl run test-curl --rm -it --restart=Never --image=curlimages/curl -- curl -v http://nginx.default.svc.cluster.local
```

**Check K6 test pod logs:**

```bash
kubectl logs -l k6_cr=nginx-load-test
```

## Performance Tuning

### Increase Load Test Intensity

Modify the K6 ConfigMap:

```javascript
export let options = {
  stages: [
    { duration: "1m", target: 500 }, // More aggressive
    { duration: "5m", target: 500 },
    { duration: "1m", target: 0 },
  ],
};
```

### Increase K6 Parallelism

More parallel K6 runners = more load:

```yaml
spec:
  parallelism: 8 # Run 8 K6 instances in parallel
```

### CPU-Intensive Test

Add more computation in the test:

```javascript
export default function () {
  let response = http.get("http://nginx.default.svc.cluster.local");

  // CPU-intensive calculation
  let sum = 0;
  for (let i = 0; i < 100000; i++) {
    sum += Math.sqrt(i) * Math.random();
  }

  sleep(0.5); // Faster requests
}
```

## References

- [K6 Operator Documentation](https://github.com/grafana/k6-operator)
- [K6 Load Testing Guide](https://k6.io/docs/)
- [Kubernetes HPA Documentation](https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/)
- [Metrics Server Documentation](https://github.com/kubernetes-sigs/metrics-server)
