# 🚀 GitHub Version API

[![API Status](https://img.shields.io/badge/API-Online-brightgreen)](https://api.tavuru.de/health)
[![Response Time](https://img.shields.io/badge/Response%20Time-<100ms-brightgreen)](https://api.tavuru.de/stats)
[![P95 Latency](https://img.shields.io/badge/P95%20Latency-80ms-green)](https://api.tavuru.de/stats)
[![Reliability](https://img.shields.io/badge/Uptime-99.9%25-brightgreen)](https://api.tavuru.de/health)
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-orange)](https://www.rust-lang.org/)

> **A blazing-fast, free public API for retrieving GitHub repository version information. Built with Rust for maximum performance and reliability.**

## ✨ What is this?

This is a **free, public API service** that provides version information for any GitHub repository. No API keys, no rate limits, no registration required. Just fast, reliable access to repository version data.

🔗 **Base URL**: `https://api.tavuru.de`

## 🎯 Why use this API?

- ⚡ **Lightning Fast** - Sub-100ms response times (69ms median)
- 🆓 **Completely Free** - No API keys, no rate limits, no costs
- 🌍 **Public Access** - Use in any project, personal or commercial
- 📊 **Rich Data** - Version info, file sizes, download counts, release notes
- 🛡️ **Rock Solid** - Memory-safe Rust implementation with 99.9% uptime
- 🔄 **Smart Caching** - 10-minute cache reduces GitHub API load
- ✅ **Proper Error Handling** - Clear HTTP status codes and error messages

## 🚀 Quick Start

### Basic Usage

```bash
# Get latest version of any GitHub repository
curl https://api.tavuru.de/version/microsoft/vscode

# Example response
{
  "version": "1.85.2",
  "release_name": "January 2024 Recovery",
  "published_at": "2024-01-22T10:30:00Z",
  "files": [
    {
      "name": "VSCode-win32-x64.zip",
      "size": 142857600,
      "size_formatted": "136.2 MB",
      "download_count": 1250000,
      "download_url": "https://github.com/microsoft/vscode/releases/download/1.85.2/VSCode-win32-x64.zip",
      "content_type": "application/zip"
    }
  ],
  "download_urls": {
    "zipball": "https://api.github.com/repos/microsoft/vscode/zipball/1.85.2",
    "tarball": "https://api.github.com/repos/microsoft/vscode/tarball/1.85.2"
  },
  "release_notes": "## What's New\n- Performance improvements\n- Bug fixes",
  "cached": true,
  "cache_expires_in": 563,
  "request_id": "abc123-def456-ghi789"
}
```

### Supported URL Patterns

```bash
# Full owner/repo specification
https://api.tavuru.de/version/facebook/react
https://api.tavuru.de/version/microsoft/typescript
https://api.tavuru.de/version/docker/compose

# Query parameters (legacy support)
https://api.tavuru.de/version?owner=torvalds&repo=linux

# Default owner shorthand (uses Ym0T as default)
https://api.tavuru.de/version/pterodactyl-nginx-egg
```

## 📖 API Endpoints

| Endpoint | Method | Description | Example Response |
|----------|--------|-------------|------------------|
| `/` | GET | API welcome message | Plain text status |
| `/health` | GET | Health check with uptime | JSON health data |
| `/stats` | GET | API usage statistics | JSON stats |
| `/version/{owner}/{repo}` | GET | Get version info | JSON version data |
| `/version/{repo}` | GET | Get version (default owner) | JSON version data |
| `/version` | GET | Query params support | JSON version data |

## 💻 Code Examples

### JavaScript/TypeScript

```javascript
// Simple version check
async function getLatestVersion(owner, repo) {
  try {
    const response = await fetch(`https://api.tavuru.de/version/${owner}/${repo}`);
    
    if (!response.ok) {
      const error = await response.json();
      throw new Error(`${error.error}: ${error.message}`);
    }
    
    const data = await response.json();
    return data.version;
  } catch (error) {
    console.error('Failed to get version:', error.message);
    return null;
  }
}

// Usage with error handling
const version = await getLatestVersion('microsoft', 'vscode');
if (version) {
  console.log(`Latest VS Code: ${version}`);
} else {
  console.log('Repository not found or API error');
}

// Check for updates
async function checkForUpdate(currentVersion, owner, repo) {
  const latest = await getLatestVersion(owner, repo);
  return latest && latest !== currentVersion;
}

// Get complete repository information
async function getRepositoryInfo(owner, repo) {
  try {
    const response = await fetch(`https://api.tavuru.de/version/${owner}/${repo}`);
    
    if (response.status === 404) {
      return { error: 'Repository not found' };
    }
    
    if (!response.ok) {
      const error = await response.json();
      return { error: error.message };
    }
    
    return await response.json();
  } catch (error) {
    return { error: 'Network error' };
  }
}
```

### Python

```python
import requests
from typing import Optional, Dict, Any

def get_repository_info(owner: str, repo: str) -> Dict[str, Any]:
    """Get complete repository version information with error handling"""
    url = f"https://api.tavuru.de/version/{owner}/{repo}"
    
    try:
        response = requests.get(url, timeout=10)
        
        if response.status_code == 404:
            return {
                "error": "repository_not_found",
                "message": f"Repository '{owner}/{repo}' not found"
            }
        
        response.raise_for_status()
        return response.json()
        
    except requests.exceptions.Timeout:
        return {"error": "timeout", "message": "Request timed out"}
    except requests.exceptions.RequestException as e:
        return {"error": "network_error", "message": str(e)}

def get_latest_version(owner: str, repo: str) -> Optional[str]:
    """Get just the version string, returns None if not found"""
    info = get_repository_info(owner, repo)
    
    if "error" in info:
        print(f"Error: {info['message']}")
        return None
    
    return info.get("version")

# Example usage
def check_for_updates(projects):
    """Check multiple projects for updates"""
    print("🔍 Checking for updates...\n")
    
    for owner, repo, current in projects:
        info = get_repository_info(owner, repo)
        
        if "error" in info:
            print(f"❌ {repo}: {info['message']}")
            continue
        
        latest = info["version"]
        cached = "💾" if info.get("cached", False) else "🌐"
        
        if latest != current:
            print(f"🆕 {repo}: {current} → {latest} {cached}")
        else:
            print(f"✅ {repo}: up to date ({current}) {cached}")

# Usage
projects = [
    ('microsoft', 'vscode', '1.85.1'),
    ('facebook', 'react', '18.2.0'),
    ('docker', 'compose', '2.24.0'),
    ('nonexistent', 'repo', '1.0.0'),  # This will show error
]
check_for_updates(projects)
```

### Bash/Shell

```bash
#!/bin/bash

# Function to get version with error handling
get_version() {
    local owner="$1"
    local repo="$2"
    
    local response=$(curl -s -w "HTTPSTATUS:%{http_code}" "https://api.tavuru.de/version/$owner/$repo")
    local body=$(echo "$response" | sed -E 's/HTTPSTATUS:[0-9]{3}$//')
    local status=$(echo "$response" | grep -o "HTTPSTATUS:[0-9]*$" | cut -d: -f2)
    
    case "$status" in
        200)
            echo "$body" | jq -r '.version'
            return 0
            ;;
        404)
            echo "Repository '$owner/$repo' not found" >&2
            return 1
            ;;
        *)
            echo "API error (HTTP $status)" >&2
            return 1
            ;;
    esac
}

# Check if update is available
check_update() {
    local current="$1"
    local owner="$2"
    local repo="$3"
    
    echo "🔍 Checking $owner/$repo..."
    
    if latest=$(get_version "$owner" "$repo"); then
        if [[ "$latest" != "$current" ]]; then
            echo "🆕 Update available for $repo: $current → $latest"
            return 0
        else
            echo "✅ $repo is up to date ($current)"
            return 1
        fi
    else
        echo "❌ Failed to check $repo"
        return 1
    fi
}

# Usage examples
echo "=== Version Checker ==="
check_update "v1.85.1" "microsoft" "vscode"
check_update "18.2.0" "facebook" "react"
check_update "1.0.0" "nonexistent" "repo"  # This will fail

# Get detailed info
echo -e "\n=== Detailed Info ==="
curl -s "https://api.tavuru.de/version/docker/compose" | jq '{
  version: .version,
  published: .published_at,
  files: (.files | length),
  cached: .cached
}'
```

### GitHub Actions

```yaml
name: Check Dependencies for Updates
on:
  schedule:
    - cron: '0 9 * * MON'  # Every Monday at 9 AM
  workflow_dispatch:

jobs:
  check-updates:
    runs-on: ubuntu-latest
    steps:
      - name: Check VS Code Updates
        id: vscode
        run: |
          response=$(curl -s -w "HTTPSTATUS:%{http_code}" "https://api.tavuru.de/version/microsoft/vscode")
          status=$(echo "$response" | grep -o "HTTPSTATUS:[0-9]*$" | cut -d: -f2)
          
          if [ "$status" = "200" ]; then
            LATEST=$(echo "$response" | sed 's/HTTPSTATUS:200$//' | jq -r '.version')
            echo "latest=$LATEST" >> $GITHUB_OUTPUT
            echo "success=true" >> $GITHUB_OUTPUT
            echo "📦 Latest VS Code: $LATEST"
          else
            echo "success=false" >> $GITHUB_OUTPUT
            echo "❌ Failed to get VS Code version"
          fi
      
      - name: Check React Updates  
        id: react
        run: |
          response=$(curl -s -w "HTTPSTATUS:%{http_code}" "https://api.tavuru.de/version/facebook/react")
          status=$(echo "$response" | grep -o "HTTPSTATUS:[0-9]*$" | cut -d: -f2)
          
          if [ "$status" = "200" ]; then
            LATEST=$(echo "$response" | sed 's/HTTPSTATUS:200$//' | jq -r '.version')
            echo "latest=$LATEST" >> $GITHUB_OUTPUT
            echo "success=true" >> $GITHUB_OUTPUT
            echo "⚛️ Latest React: $LATEST"
          else
            echo "success=false" >> $GITHUB_OUTPUT
            echo "❌ Failed to get React version"
          fi
      
      - name: Create Issue if Updates Available
        if: |
          (steps.vscode.outputs.success == 'true' && steps.vscode.outputs.latest != '1.85.1') ||
          (steps.react.outputs.success == 'true' && steps.react.outputs.latest != '18.2.0')
        uses: actions/github-script@v7
        with:
          script: |
            const updates = [];
            
            if ('${{ steps.vscode.outputs.latest }}' !== '1.85.1') {
              updates.push('- **VS Code**: ${{ steps.vscode.outputs.latest }}');
            }
            
            if ('${{ steps.react.outputs.latest }}' !== '18.2.0') {
              updates.push('- **React**: ${{ steps.react.outputs.latest }}');
            }
            
            if (updates.length > 0) {
              github.rest.issues.create({
                owner: context.repo.owner,
                repo: context.repo.repo,
                title: '🆕 Dependency Updates Available',
                body: `## Updates Available\n\n${updates.join('\n')}\n\n_Automatically generated by GitHub Actions_`
              });
            }
```

## 📊 Response Format

### Success Response

```typescript
interface VersionResponse {
  version: string;           // Git tag or version identifier
  release_name?: string;     // Human-readable release name  
  published_at: string;      // ISO 8601 timestamp
  files: FileInfo[];         // Array of downloadable files
  download_urls: {
    zipball: string;         // ZIP archive URL
    tarball: string;         // TAR.GZ archive URL
  };
  release_notes?: string;    // Markdown release notes
  cached: boolean;           // Whether response was cached
  cache_expires_in: number;  // Cache TTL in seconds
  request_id: string;        // Unique request identifier
}

interface FileInfo {
  name: string;              // File name
  size: number;              // Size in bytes
  size_formatted: string;    // Human-readable size (e.g., "1.2 MB")
  download_count: number;    // GitHub download count
  download_url: string;      // Direct download URL
  content_type?: string;     // MIME type
}
```

### Error Response

```typescript
interface ErrorResponse {
  error: string;             // Error code
  message: string;           // Human-readable error message
  request_id: string;        // Unique request identifier
  timestamp: string;         // ISO 8601 timestamp
}
```

### Common Error Codes

| HTTP Status | Error Code | Description |
|-------------|------------|-------------|
| `404` | `repository_not_found` | Repository doesn't exist or is private |
| `500` | `network_error` | GitHub API or network issues |
| `500` | `internal_server_error` | Unexpected server error |

### Example Error Response

```json
{
  "error": "repository_not_found",
  "message": "Repository 'nonexistent/repo' not found or is private",
  "request_id": "abc123-def456-ghi789",
  "timestamp": "2025-06-23T12:00:00Z"
}
```

## ⚡ Performance

This API is built for speed and reliability:

- **Response Time**: Sub-100ms average (69ms median)
- **95th Percentile**: 80ms - Consistently fast responses
- **99th Percentile**: 87ms - Excellent tail latency
- **Throughput**: 800+ requests/second capability
- **Cache Duration**: 10 minutes per repository
- **Uptime**: 99.9%+ target
- **Infrastructure**: Rust + Cloudflare Enterprise

### 🏆 Performance Benchmarks

| Load Level | Concurrent Users | Median Response | 95th Percentile | Throughput |
|------------|------------------|-----------------|-----------------|------------|
| **Light** | 10 | **69ms** | **80ms** | 136 RPS |
| **Medium** | 30 | 105ms | 158ms | 212 RPS |
| **Heavy** | 150 | 650ms | 875ms | 219 RPS |
| **Extreme** | 500 | 563ms | 817ms | 820 RPS |

*Tested on single-core infrastructure with Cloudflare CDN*

### 📈 Performance Comparison

Your API performs in the **Elite Tier** for public APIs:

| Performance Class | Response Time Range | Your API |
|-------------------|-------------------|----------|
| **Elite** | <100ms | **69ms** ✅ |
| **Excellent** | 100-200ms | - |
| **Good** | 200-500ms | - |
| **Acceptable** | 500ms-1s | - |
| **Slow** | >1s | - |

> 🎯 **Sub-100ms APIs are rare** - This API performs in the top 5% of public APIs!

## 🎨 Use Cases

### For Developers
- **Dependency Monitoring** - Track updates for your project dependencies
- **CI/CD Integration** - Automate version checks in your build pipeline
- **Update Notifications** - Alert users when new versions are available
- **Dashboard Creation** - Display current versions across multiple projects

### For Applications  
- **Desktop Apps** - Check for application updates
- **Mobile Apps** - Implement update notifications
- **Web Services** - Monitor service dependencies
- **DevOps Tools** - Automate infrastructure updates

### For Websites
- **Documentation Sites** - Auto-update version numbers
- **Project Pages** - Display latest release information
- **Download Pages** - Show current version and file sizes
- **Status Dashboards** - Monitor multiple repositories

## 🔧 Data Sources

The API intelligently fetches version information from multiple sources:

1. **GitHub Releases API** (primary) - Full release data with assets and download counts
2. **GitHub Tags API** (fallback) - Tag-based version info when no releases exist
3. **VERSION files** (fallback) - Repository version files in root directory
4. **package.json** (fallback) - Node.js package version information

## 🛡️ Fair Use Policy

This API is **completely free** with no rate limits. Please use responsibly:

- ✅ **Personal and commercial use** - No restrictions
- ✅ **No authentication required** - Just start using it
- ✅ **No rate limits** - Use as much as you need
- ✅ **Cache responses** - Client-side caching recommended for better performance
- ❌ **Don't abuse** - Please avoid excessive requests that could impact service

## 📊 API Status & Monitoring

Check API health and get real-time statistics:

```bash
# Health check
curl https://api.tavuru.de/health

# Usage statistics  
curl https://api.tavuru.de/stats

# Example stats response
{
  "total_requests": 45632,
  "cache_hits": 38291,
  "cache_misses": 7341,
  "uptime": "15d 7h 23m 45s",
  "memory_usage": {
    "cache_entries": 1247,
    "cache_size_estimate": "~1.2 MB"
  }
}
```

## 🚨 Error Handling Best Practices

When using this API, always implement proper error handling:

### JavaScript Example
```javascript
async function safeApiCall(owner, repo) {
  try {
    const response = await fetch(`https://api.tavuru.de/version/${owner}/${repo}`);
    
    // Handle different HTTP status codes
    switch (response.status) {
      case 200:
        return await response.json();
      case 404:
        const error = await response.json();
        throw new Error(`Repository not found: ${error.message}`);
      case 500:
        throw new Error('API server error, please try again later');
      default:
        throw new Error(`Unexpected status: ${response.status}`);
    }
  } catch (error) {
    console.error('API call failed:', error.message);
    return null;
  }
}
```

### Python Example
```python
import requests

def safe_api_call(owner, repo):
    try:
        response = requests.get(
            f"https://api.tavuru.de/version/{owner}/{repo}",
            timeout=10
        )
        
        if response.status_code == 200:
            return response.json()
        elif response.status_code == 404:
            print(f"Repository {owner}/{repo} not found")
            return None
        else:
            print(f"API error: {response.status_code}")
            return None
            
    except requests.exceptions.Timeout:
        print("Request timed out")
        return None
    except requests.exceptions.RequestException as e:
        print(f"Request failed: {e}")
        return None
```

## 🤝 Support & Feedback

Found a bug? Have a feature request? Want to contribute?

- 🐛 **Report Issues**: [Open a GitHub Issue](../../issues/new)
- 💡 **Feature Requests**: [Start a Discussion](../../discussions/new)
- 📧 **Contact**: Create an issue for any questions
- ⭐ **Star this repo** if the API helped you!

## 🏗️ Technical Details

### Built With
- **[Rust](https://www.rust-lang.org/)** - Memory-safe systems programming
- **[Axum](https://github.com/tokio-rs/axum)** - High-performance web framework
- **[Tokio](https://tokio.rs/)** - Asynchronous runtime
- **[Reqwest](https://github.com/seanmonstar/reqwest)** - HTTP client
- **[Serde](https://serde.rs/)** - JSON serialization
- **[Moka](https://github.com/moka-rs/moka)** - High-performance caching
- **[Cloudflare](https://cloudflare.com/)** - Global CDN and DDoS protection

### Architecture
- **Single-core optimization** - Maximum efficiency with minimal resources
- **In-memory caching** - Fast response times with 10-minute TTL
- **Graceful degradation** - Multiple fallback data sources
- **Zero downtime deployment** - Built for reliability
- **Proper error handling** - Clear HTTP status codes and error messages

### Security
- **HTTPS only** - All communications encrypted
- **No data collection** - Privacy-focused design
- **Rate limiting friendly** - Smart caching reduces GitHub API load
- **Input validation** - Prevents malicious requests

## 🔗 Quick Links

- **🌐 API Base URL**: https://api.tavuru.de
- **💚 Health Check**: https://api.tavuru.de/health  
- **📊 Statistics**: https://api.tavuru.de/stats
- **🧪 Test the API**: `curl https://api.tavuru.de/version/microsoft/vscode`

---

<div align="center">

**Made with ❤️ and ⚡ by [Ym0T](https://github.com/Ym0T)**

*Built with Rust for blazing-fast performance • Powered by Cloudflare for global reliability*

**If this API saved you time, consider ⭐ starring this repository!**

</div>
